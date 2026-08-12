//! OpenAI Responses API 客户端（`/responses` 端点，DeepSeek 优先）。
//!
//! 与 Chat Completions 客户端（[`super::zai::ZaiClient`]）的差异：
//! - 请求 `input` 是结构化 item 数组（见 [`crate::responses_items`]）；
//! - 流式是 `response.*` 语义事件序列，无 `[DONE]`（见 [`crate::responses_stream`]）；
//! - reasoning 以独立 item 明文回传（DeepSeek 归并到相邻 assistant 消息），
//!   不再需要 `reasoning_content` 字段回放 hack；
//! - `previous_response_id` 链式续接预留：DeepSeek `/responses` 当前**无状态**
//!   （官方文档：`previous_response_id` / `store` 不支持），`supports_chaining`
//!   默认关闭；未来支持的端点可经 [`OpenAiResponsesClient::with_chaining`] 打开，
//!   链失效（hash 失配 / 服务端驱逐）自动回退全量请求。

use crate::http_client::{build_api_http_client, build_streaming_api_http_client};
use crate::providers::zai::{
    is_quota_exhausted, is_retryable_status, openai_compatible_reasoning_effort,
    openai_compatible_thinking_body, provider_label_from_config, retry_delay_ms,
    sanitize_header_token, DEFAULT_MAX_RETRIES,
};
use crate::responses_items::{
    chain_plan, chain_state_json, hash_prefix, is_chain_miss, messages_to_responses_input,
    responses_tools_from_schemas, ChainPlan,
};
use crate::responses_stream::ResponsesSseStreamState;
use crate::sse_data_lines::{SseDataLine, SseLineBuffer};
use anycode_core::prelude::*;
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// DeepSeek Responses API 端点（无状态；v4-flash 先行，v4-pro 2026-08 起支持）。
pub const DEEPSEEK_RESPONSES_URL: &str = "https://api.deepseek.com/responses";
pub const DEEPSEEK_RESPONSES_DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// OpenAI Responses API 客户端。
pub struct OpenAiResponsesClient {
    client: Client,
    /// 流式专用：无总超时，仅 connect + 读空闲超时（见 `crate::http_client`）。
    stream_client: Client,
    api_key: String,
    base_url: String,
    model: String,
    /// 端点是否支持 `previous_response_id` 链式（DeepSeek：否）。
    supports_chaining: bool,
}

impl OpenAiResponsesClient {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            client: build_api_http_client(),
            stream_client: build_streaming_api_http_client(),
            api_key,
            base_url: DEEPSEEK_RESPONSES_URL.to_string(),
            model: model.unwrap_or_else(|| DEEPSEEK_RESPONSES_DEFAULT_MODEL.to_string()),
            supports_chaining: false,
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        let trimmed = base_url.trim().trim_end_matches('/');
        self.base_url = if trimmed.ends_with("/responses") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/responses")
        };
        self
    }

    #[allow(dead_code)] // 预留：DeepSeek 开放 previous_response_id / 接入官方 OpenAI 时打开
    pub fn with_chaining(mut self, enabled: bool) -> Self {
        self.supports_chaining = enabled;
        self
    }
}

/// DeepSeek thinking / effort 字段（Responses 形状：`reasoning.effort`；
/// 思考开关未在兼容表中列出，`thinking` 字段发送后被静默忽略，作兜底）。
fn apply_reasoning_fields(provider_label: &str, config: &ModelConfig, body: &mut Value) {
    if !provider_label.starts_with("deepseek") {
        return;
    }
    let thinking = openai_compatible_thinking_body("deepseek", config);
    match thinking {
        Some(_) => {
            if let Some(effort) = openai_compatible_reasoning_effort("deepseek", config) {
                body["reasoning"] = json!({ "effort": effort });
            }
        }
        None => {
            body["thinking"] = json!({ "type": "disabled" });
        }
    }
}

/// 组装请求体；链式可行时同时返回 (chained, full) 两个候选。
#[allow(clippy::too_many_arguments)]
fn build_bodies(
    client: &OpenAiResponsesClient,
    messages: &[Message],
    tools: &[ToolSchema],
    config: &ModelConfig,
    provider_label: &str,
    stream: bool,
) -> Result<(Value, Option<Value>), CoreError> {
    let model = if config.model.trim().is_empty() {
        client.model.clone()
    } else {
        config.model.clone()
    };

    let mut base = json!({
        "model": model,
        "stream": stream,
    });
    if let Some(t) = config.temperature {
        base["temperature"] = json!(t);
    }
    if let Some(m) = config.max_tokens {
        base["max_output_tokens"] = json!(m);
    }
    if !tools.is_empty() {
        base["tools"] = json!(responses_tools_from_schemas(tools));
        base["tool_choice"] = json!("auto");
    }
    apply_reasoning_fields(provider_label, config, &mut base);

    let mut full = base.clone();
    full["input"] = json!(messages_to_responses_input(messages)?);

    let chained = match chain_plan(messages, client.supports_chaining) {
        ChainPlan::Full => None,
        ChainPlan::Chained {
            previous_response_id,
            tail_start,
        } => {
            let mut c = base;
            c["previous_response_id"] = json!(previous_response_id);
            c["input"] = json!(messages_to_responses_input(&messages[tail_start..])?);
            info!(
                tail_items = messages.len() - tail_start,
                "responses chain=chained"
            );
            Some(c)
        }
    };
    Ok((full, chained))
}

/// 带重试的发送；链式请求遇 chain-miss（服务端驱逐 previous response）时
/// 透明降级为全量请求重发一次。
async fn send_with_retries(
    client: &Client,
    url: &str,
    auth_key: &str,
    provider_label: &str,
    full_body: &Value,
    chained_body: Option<&Value>,
) -> Result<reqwest::Response, CoreError> {
    let mut chained = chained_body;
    #[allow(unused_assignments)]
    let mut last_err = String::from("unknown error");
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let body = chained.unwrap_or(full_body);
        let send_res = client
            .post(url)
            .header("Authorization", format!("Bearer {}", auth_key))
            .json(body)
            .send()
            .await;

        match send_res {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(resp);
                }
                let retry_after_ms = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|secs| secs.saturating_mul(1000));
                let error_text = resp.text().await.unwrap_or_default();
                let snippet = &error_text[..error_text.len().min(500)];

                if chained.is_some() && is_chain_miss(status.as_u16(), &error_text) {
                    warn!("{provider_label} responses chain miss (server evicted) -> retry stateless once");
                    chained = None;
                    continue;
                }

                last_err = format!(
                    "{} API error: status={} url={} body={}",
                    provider_label,
                    status.as_u16(),
                    url,
                    if snippet.is_empty() {
                        "<empty>"
                    } else {
                        snippet
                    }
                );
                if let Some(hint) = crate::providers::zai::billing_failure_hint(status, &error_text)
                {
                    last_err = format!("{last_err} · {hint}");
                }
                if is_quota_exhausted(&error_text) {
                    error!("{provider_label} quota exhausted — failing fast without retries");
                    break;
                }
                if attempt <= DEFAULT_MAX_RETRIES && is_retryable_status(status) {
                    let delay = retry_after_ms.unwrap_or_else(|| retry_delay_ms(attempt));
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                    continue;
                }
                break;
            }
            Err(e) => {
                last_err = e.to_string();
                if attempt <= DEFAULT_MAX_RETRIES {
                    tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms(attempt)))
                        .await;
                    continue;
                }
                break;
            }
        }
    }
    Err(CoreError::LLMError(format!(
        "{} request failed after retries: {}",
        provider_label, last_err
    )))
}

/// 解析非流式 Responses 响应对象。
fn llm_response_from_responses_value(
    v: &Value,
    tools: &[ToolSchema],
    chain_meta: Option<Value>,
) -> Result<LLMResponse, CoreError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut native: Vec<ToolCall> = Vec::new();

    for item in v
        .get("output")
        .and_then(|o| o.as_array())
        .cloned()
        .unwrap_or_default()
    {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                for part in item
                    .get("content")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default()
                {
                    if part.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                        if let Some(t) = part.get("text").and_then(|x| x.as_str()) {
                            text.push_str(t);
                        }
                    }
                }
            }
            Some("reasoning") => {
                for part in item
                    .get("content")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default()
                {
                    if let Some(t) = part.get("text").and_then(|x| x.as_str()) {
                        reasoning.push_str(t);
                    }
                }
            }
            Some("function_call") => {
                let args = item.get("arguments").and_then(|x| x.as_str()).unwrap_or("");
                native.push(ToolCall {
                    id: item
                        .get("call_id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    input: crate::tool_call_normalizer::parse_arguments_value(args),
                });
            }
            _ => {}
        }
    }

    let normalized = crate::tool_call_normalizer::normalize_assistant_output(native, &text, tools);

    let mut metadata = std::collections::HashMap::new();
    if !reasoning.trim().is_empty() {
        metadata.insert(
            ANYCODE_REASONING_CONTENT_METADATA_KEY.to_string(),
            json!(reasoning),
        );
    }
    if let Some(cm) = chain_meta {
        metadata.insert(ANYCODE_RESPONSE_ID_METADATA_KEY.to_string(), cm);
    }

    let usage = v.get("usage").cloned().unwrap_or(Value::Null);
    Ok(LLMResponse {
        message: Message {
            id: Uuid::new_v4(),
            role: MessageRole::Assistant,
            content: MessageContent::Text(normalized.visible_content),
            timestamp: chrono::Utc::now(),
            metadata,
        },
        tool_calls: normalized.tool_calls,
        usage: Usage {
            input_tokens: usage
                .get("input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as u32,
            output_tokens: usage
                .get("output_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as u32,
            cache_creation_tokens: None,
            cache_read_tokens: usage
                .get("input_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|x| x.as_u64())
                .map(|v| v as u32),
        },
    })
}

#[async_trait]
impl LLMClient for OpenAiResponsesClient {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        config: &ModelConfig,
    ) -> Result<LLMResponse, CoreError> {
        let provider_label = provider_label_from_config(config);
        let (full_body, chained_body) =
            build_bodies(self, &messages, &tools, config, &provider_label, false)?;

        let base_url = if config.base_url.as_deref().unwrap_or("").is_empty() {
            self.base_url.clone()
        } else {
            let u = config.base_url.clone().unwrap_or_default();
            let t = u.trim().trim_end_matches('/');
            if t.ends_with("/responses") {
                t.to_string()
            } else {
                format!("{t}/responses")
            }
        };
        let auth_key = config
            .api_key
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(self.api_key.as_str());
        let auth_key = sanitize_header_token(auth_key, &provider_label)?;

        let chaining_active = chained_body.is_some();
        let response = send_with_retries(
            &self.client,
            &base_url,
            &auth_key,
            &provider_label,
            &full_body,
            chained_body.as_ref(),
        )
        .await?;

        let v: Value = response
            .json()
            .await
            .map_err(|e| CoreError::LLMError(e.to_string()))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            return Err(CoreError::LLMError(format!(
                "{provider_label} API error: {err}"
            )));
        }

        let chain_meta = if chaining_active || self.supports_chaining {
            v.get("id")
                .and_then(|x| x.as_str())
                .map(|id| chain_state_json(id, &hash_prefix(&messages)))
        } else {
            None
        };
        llm_response_from_responses_value(&v, &tools, chain_meta)
    }

    async fn chat_stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        config: &ModelConfig,
    ) -> Result<mpsc::Receiver<StreamEvent>, CoreError> {
        let provider_label = provider_label_from_config(config);
        let (full_body, chained_body) =
            build_bodies(self, &messages, &tools, config, &provider_label, true)?;

        let prefix_hash = if self.supports_chaining {
            hash_prefix(&messages)
        } else {
            String::new()
        };

        let base_url = if config.base_url.as_deref().unwrap_or("").is_empty() {
            self.base_url.clone()
        } else {
            let u = config.base_url.clone().unwrap_or_default();
            let t = u.trim().trim_end_matches('/');
            if t.ends_with("/responses") {
                t.to_string()
            } else {
                format!("{t}/responses")
            }
        };
        let auth_key = config
            .api_key
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(self.api_key.as_str());
        let auth_key = sanitize_header_token(auth_key, &provider_label)?;

        let client = self.stream_client.clone();
        let (tx, rx) = mpsc::channel(128);

        tokio::spawn(async move {
            let response = match send_with_retries(
                &client,
                &base_url,
                &auth_key,
                &provider_label,
                &full_body,
                chained_body.as_ref(),
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("{} stream request failed: {}", provider_label, e);
                    let _ = tx.send(StreamEvent::Failed(e.to_string())).await;
                    return;
                }
            };

            let mut stream = response.bytes_stream();
            let mut sse_buf = SseLineBuffer::new();
            let mut state = ResponsesSseStreamState::new(prefix_hash);
            // 终止标记：response.completed / response.incomplete（failed 经 Failed 事件上报）。
            // DeepSeek Responses 流没有 [DONE]；无终止事件即中途截断。
            let mut read_failure: Option<String> = None;
            let mut failed_sent = false;

            while let Some(chunk_res) = stream.next().await {
                let chunk = match chunk_res {
                    Ok(c) => c,
                    Err(e) => {
                        error!("{} stream read: {}", provider_label, e);
                        read_failure = Some(e.to_string());
                        break;
                    }
                };
                let Ok(text) = std::str::from_utf8(&chunk) else {
                    continue;
                };
                for line_ev in sse_buf.push_str(text) {
                    let SseDataLine::Payload(data) = line_ev else {
                        continue;
                    };
                    let Ok(events) = state.push_json_str(&data) else {
                        continue;
                    };
                    for ev in events {
                        let is_failed = matches!(ev, StreamEvent::Failed(_));
                        if tx.send(ev).await.is_err() {
                            return;
                        }
                        if is_failed {
                            failed_sent = true;
                        }
                    }
                    if failed_sent {
                        return;
                    }
                }
            }
            for line_ev in sse_buf.finish() {
                let SseDataLine::Payload(data) = line_ev else {
                    continue;
                };
                let Ok(events) = state.push_json_str(&data) else {
                    continue;
                };
                for ev in events {
                    let is_failed = matches!(ev, StreamEvent::Failed(_));
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                    if is_failed {
                        failed_sent = true;
                    }
                }
            }
            if failed_sent {
                return;
            }

            if let Some(err) = read_failure {
                let _ = tx
                    .send(StreamEvent::Failed(format!(
                        "{provider_label} stream read interrupted: {err}"
                    )))
                    .await;
                return;
            }
            if !state.is_complete() {
                let _ = tx
                    .send(StreamEvent::Failed(format!(
                        "{provider_label} stream ended before response.completed"
                    )))
                    .await;
                return;
            }

            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_non_stream_response_with_reasoning_and_tool_call() {
        let v = json!({
            "id": "resp_1",
            "output": [
                { "type": "reasoning", "content": [{ "type": "reasoning_text", "text": "think" }] },
                { "type": "message", "content": [{ "type": "output_text", "text": "checking" }] },
                { "type": "function_call", "call_id": "call_1", "name": "Read", "arguments": "{\"p\":1}" }
            ],
            "usage": {
                "input_tokens": 50,
                "output_tokens": 10,
                "input_tokens_details": { "cached_tokens": 32 }
            }
        });
        let resp = llm_response_from_responses_value(&v, &[], None).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_1");
        assert_eq!(resp.tool_calls[0].input, json!({"p": 1}));
        assert_eq!(resp.usage.input_tokens, 50);
        assert_eq!(resp.usage.cache_read_tokens, Some(32));
        assert_eq!(
            resp.message
                .metadata
                .get(ANYCODE_REASONING_CONTENT_METADATA_KEY)
                .and_then(|x| x.as_str()),
            Some("think")
        );
        if let MessageContent::Text(t) = &resp.message.content {
            assert_eq!(t, "checking");
        } else {
            panic!("expected text");
        }
    }

    #[test]
    fn with_base_url_appends_responses_suffix() {
        let c = OpenAiResponsesClient::new("k".into(), None)
            .with_base_url("https://api.deepseek.com/".into());
        assert_eq!(c.base_url, "https://api.deepseek.com/responses");
        let c = OpenAiResponsesClient::new("k".into(), None)
            .with_base_url("https://example.com/v1/responses".into());
        assert_eq!(c.base_url, "https://example.com/v1/responses");
    }
}
