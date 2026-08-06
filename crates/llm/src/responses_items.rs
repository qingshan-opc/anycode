//! OpenAI Responses API 输入 item 序列化与链式续接规划（纯函数，全量单测）。
//!
//! 与 Chat Completions 的差异：
//! - 输入是结构化 item 数组（message / reasoning / function_call / function_call_output），
//!   而非扁平 messages；
//! - reasoning 是独立 item，明文回传即可（DeepSeek 会归并到相邻 assistant 消息），
//!   不再需要 chat-completions 的 `reasoning_content` 字段 hack；
//! - 支持 `previous_response_id` 的端点可链式续接（服务端保存前缀状态，只发增量）。
//!   DeepSeek `/responses` 目前**无状态**（不支持 previous_response_id/store），
//!   链式默认关闭，仅对未来支持的端点经 [`ChainPlan`] 打开。

use anycode_core::prelude::*;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Assistant reasoning 以 Responses reasoning item 明文回传（DeepSeek：归并到相邻 assistant）。
fn reasoning_item(text: &str) -> Value {
    json!({
        "type": "reasoning",
        "content": [{ "type": "reasoning_text", "text": text }]
    })
}

/// User message content parts（文本 + 可选图片）。DeepSeek 不支持 input_image
///（替换为占位文本），但通用 Responses 端点接受，按官方格式发送。
fn user_content_parts(msg: &Message, text: &str) -> Value {
    let images = anycode_core::vision_images_from_metadata(&msg.metadata);
    if images.is_empty() {
        return json!([{ "type": "input_text", "text": text }]);
    }
    let mut parts: Vec<Value> = Vec::new();
    if !text.is_empty() {
        parts.push(json!({ "type": "input_text", "text": text }));
    }
    for img in images {
        parts.push(json!({
            "type": "input_image",
            "image_url": format!("data:{};base64,{}", img.mime_type, img.data_base64)
        }));
    }
    if parts.is_empty() {
        parts.push(json!({ "type": "input_text", "text": text }));
    }
    Value::Array(parts)
}

fn assistant_reasoning_text(msg: &Message) -> Option<&str> {
    msg.metadata
        .get(ANYCODE_REASONING_CONTENT_METADATA_KEY)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
}

fn assistant_tool_calls(msg: &Message) -> Result<Vec<ToolCall>, CoreError> {
    let Some(raw) = msg.metadata.get(ANYCODE_TOOL_CALLS_METADATA_KEY) else {
        return Ok(vec![]);
    };
    serde_json::from_value(raw.clone()).map_err(|e| {
        CoreError::LLMError(format!(
            "invalid {} metadata: {}",
            ANYCODE_TOOL_CALLS_METADATA_KEY, e
        ))
    })
}

/// 将 anyCode 消息历史转为 Responses API `input` item 数组（无状态全量）。
pub(crate) fn messages_to_responses_input(messages: &[Message]) -> Result<Vec<Value>, CoreError> {
    let mut out = Vec::with_capacity(messages.len() * 2);
    for msg in messages {
        match msg.role {
            MessageRole::System => {
                let MessageContent::Text(text) = &msg.content else {
                    continue;
                };
                out.push(json!({
                    "type": "message",
                    "role": "system",
                    "content": [{ "type": "input_text", "text": text }]
                }));
            }
            MessageRole::User => {
                let MessageContent::Text(text) = &msg.content else {
                    continue;
                };
                out.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": user_content_parts(msg, text)
                }));
            }
            MessageRole::Assistant => {
                if let Some(rc) = assistant_reasoning_text(msg) {
                    out.push(reasoning_item(rc));
                }
                let text = match &msg.content {
                    MessageContent::Text(t) => t.clone(),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    out.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }]
                    }));
                }
                for call in assistant_tool_calls(msg)? {
                    let args =
                        serde_json::to_string(&call.input).unwrap_or_else(|_| "{}".to_string());
                    out.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": args
                    }));
                }
            }
            MessageRole::Tool => {
                let MessageContent::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = &msg.content
                else {
                    return Err(CoreError::LLMError(format!(
                        "expected ToolResult for Tool role, got {:?}",
                        msg.content
                    )));
                };
                out.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_use_id,
                    "output": content
                }));
            }
        }
    }
    Ok(out)
}

/// Responses API 扁平 tools 形状（非 chat-completions 的嵌套 `function` 包裹）。
pub(crate) fn responses_tools_from_schemas(tools: &[ToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": crate::providers::zai::normalize_tool_parameters_schema(&t.input_schema)
            })
        })
        .collect()
}

// ============================================================================
// 链式续接（previous_response_id）
// ============================================================================

/// 链式计划：`Full` = 无状态全量；`Chained` = 服务端已有前缀，只发增量 tail。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChainPlan {
    Full,
    Chained {
        previous_response_id: String,
        /// tail 在原始 messages 中的起始下标（含 ephemeral）。
        tail_start: usize,
    },
}

fn is_ephemeral(msg: &Message) -> bool {
    msg.metadata
        .get(REPLY_LANGUAGE_REMINDER_METADATA_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 对前缀消息做确定性 hash（进程内一致即可）：role + content + assistant 的
/// `anycode_tool_calls` metadata。reasoning 等其他 metadata 不参与（不影响 item 语义）。
/// ephemeral 快照消息跳过。
pub(crate) fn hash_prefix(messages: &[Message]) -> String {
    let mut h = DefaultHasher::new();
    for m in messages.iter().filter(|m| !is_ephemeral(m)) {
        match m.role {
            MessageRole::System => "system".hash(&mut h),
            MessageRole::User => "user".hash(&mut h),
            MessageRole::Assistant => "assistant".hash(&mut h),
            MessageRole::Tool => "tool".hash(&mut h),
        }
        match &m.content {
            MessageContent::Text(t) => t.hash(&mut h),
            MessageContent::ToolUse { name, input } => {
                name.hash(&mut h);
                input.to_string().hash(&mut h);
            }
            MessageContent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                tool_use_id.hash(&mut h);
                content.hash(&mut h);
                is_error.hash(&mut h);
            }
        }
        if m.role == MessageRole::Assistant {
            if let Some(tc) = m.metadata.get(ANYCODE_TOOL_CALLS_METADATA_KEY) {
                tc.to_string().hash(&mut h);
            }
        }
    }
    format!("{:016x}", h.finish())
}

/// 解析 assistant metadata 中的链式状态 `{"id","prefix_hash"}`。
pub(crate) fn parse_chain_state(msg: &Message) -> Option<(String, String)> {
    let v = msg.metadata.get(ANYCODE_RESPONSE_ID_METADATA_KEY)?;
    let id = v.get("id").and_then(|x| x.as_str())?.to_string();
    let hash = v.get("prefix_hash").and_then(|x| x.as_str())?.to_string();
    Some((id, hash))
}

/// 生成写入 assistant metadata 的链式状态 JSON。
pub(crate) fn chain_state_json(response_id: &str, prefix_hash: &str) -> Value {
    json!({ "id": response_id, "prefix_hash": prefix_hash })
}

fn chaining_killed_by_env() -> bool {
    matches!(
        std::env::var("ANYCODE_RESPONSES_CHAIN").ok().as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off")
    )
}

/// 规划本次请求是否链式续接。
///
/// `supported` 由客户端按端点能力传入（DeepSeek `/responses` 无状态 → false）。
/// 找最后一条带链式状态的 assistant，校验前缀 hash 未被改写（microcompact /
/// compaction 会改历史 → 失配回 Full，正确性优先于省 token）。
pub(crate) fn chain_plan(messages: &[Message], supported: bool) -> ChainPlan {
    if !supported || chaining_killed_by_env() {
        return ChainPlan::Full;
    }
    for i in (0..messages.len()).rev() {
        let m = &messages[i];
        if m.role != MessageRole::Assistant {
            continue;
        }
        let Some((id, stored_hash)) = parse_chain_state(m) else {
            continue;
        };
        let h = hash_prefix(&messages[..i]);
        if h == stored_hash {
            if i + 1 >= messages.len() {
                return ChainPlan::Full;
            }
            return ChainPlan::Chained {
                previous_response_id: id,
                tail_start: i + 1,
            };
        }
        tracing::info!("responses chain invalidated (history rewritten) -> full request");
        return ChainPlan::Full;
    }
    ChainPlan::Full
}

/// 判断一次 4xx 是否为「服务端链状态已失效」（TTL 驱逐 / 找不到 previous response）。
pub(crate) fn is_chain_miss(status: u16, body: &str) -> bool {
    if status != 400 {
        return false;
    }
    let b = body.to_ascii_lowercase().replace([' ', '-'], "_");
    b.contains("previous_response")
        && (b.contains("not_found")
            || b.contains("expired")
            || b.contains("deleted")
            || b.contains("invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn msg(role: MessageRole, text: &str) -> Message {
        Message {
            id: uuid::Uuid::new_v4(),
            role,
            content: MessageContent::Text(text.to_string()),
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        }
    }

    fn tool_result(id: &str, content: &str) -> Message {
        Message {
            id: uuid::Uuid::new_v4(),
            role: MessageRole::Tool,
            content: MessageContent::ToolResult {
                tool_use_id: id.to_string(),
                content: content.to_string(),
                is_error: false,
            },
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        }
    }

    fn assistant_with_calls(text: &str, calls: Vec<ToolCall>, reasoning: Option<&str>) -> Message {
        let mut m = msg(MessageRole::Assistant, text);
        m.metadata.insert(
            ANYCODE_TOOL_CALLS_METADATA_KEY.to_string(),
            serde_json::to_value(calls).unwrap(),
        );
        if let Some(r) = reasoning {
            m.metadata
                .insert(ANYCODE_REASONING_CONTENT_METADATA_KEY.to_string(), json!(r));
        }
        m
    }

    #[test]
    fn maps_system_user_assistant_tool_items() {
        let history = vec![
            msg(MessageRole::System, "sys"),
            msg(MessageRole::User, "hi"),
            assistant_with_calls(
                "let me check",
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "Read".into(),
                    input: json!({"path": "a.rs"}),
                }],
                Some("thinking…"),
            ),
            tool_result("call_1", "file body"),
        ];
        let items = messages_to_responses_input(&history).unwrap();
        assert_eq!(items.len(), 6);
        assert_eq!(items[0]["role"], "system");
        assert_eq!(items[0]["content"][0]["type"], "input_text");
        assert_eq!(items[1]["role"], "user");
        // reasoning 作为独立 item 回传，在 assistant 文本之前
        assert_eq!(items[2]["type"], "reasoning");
        assert_eq!(items[2]["content"][0]["type"], "reasoning_text");
        assert_eq!(items[2]["content"][0]["text"], "thinking…");
        assert_eq!(items[3]["type"], "message");
        assert_eq!(items[3]["role"], "assistant");
        assert_eq!(items[3]["content"][0]["type"], "output_text");
        assert_eq!(items[4]["type"], "function_call");
        assert_eq!(items[4]["call_id"], "call_1");
        assert_eq!(items[4]["arguments"], "{\"path\":\"a.rs\"}");
        assert_eq!(items[5]["type"], "function_call_output");
        assert_eq!(items[5]["call_id"], "call_1");
        assert_eq!(items[5]["output"], "file body");
    }

    #[test]
    fn assistant_without_reasoning_or_calls_emits_single_message() {
        let items = messages_to_responses_input(&[msg(MessageRole::Assistant, "ok")]).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["role"], "assistant");
    }

    #[test]
    fn tools_use_flat_function_shape() {
        let tools = vec![ToolSchema {
            name: "Read".into(),
            description: "read file".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "p": { "anyOf": [{"type":"string"},{"type":"null"}] } }
            }),
        }];
        let out = responses_tools_from_schemas(&tools);
        assert_eq!(out[0]["type"], "function");
        assert_eq!(out[0]["name"], "Read");
        assert!(out[0].get("function").is_none());
        // nullable anyOf 已折叠
        assert_eq!(out[0]["parameters"]["properties"]["p"]["type"], "string");
    }

    #[test]
    fn chain_plan_full_when_unsupported_or_no_state() {
        let history = vec![msg(MessageRole::User, "hi")];
        assert_eq!(chain_plan(&history, false), ChainPlan::Full);
        assert_eq!(chain_plan(&history, true), ChainPlan::Full);
    }

    #[test]
    fn chain_plan_chained_with_valid_state() {
        let mut history = vec![
            msg(MessageRole::User, "hi"),
            msg(MessageRole::Assistant, "hello"),
            msg(MessageRole::User, "and?"),
        ];
        let h = hash_prefix(&history[..1]);
        history[1].metadata.insert(
            ANYCODE_RESPONSE_ID_METADATA_KEY.to_string(),
            chain_state_json("resp_123", &h),
        );
        assert_eq!(
            chain_plan(&history, true),
            ChainPlan::Chained {
                previous_response_id: "resp_123".into(),
                tail_start: 2
            }
        );
    }

    #[test]
    fn chain_plan_full_when_prefix_rewritten() {
        let mut history = vec![
            msg(MessageRole::User, "hi"),
            msg(MessageRole::Assistant, "hello"),
            msg(MessageRole::User, "and?"),
        ];
        let h = hash_prefix(&history[..1]);
        history[1].metadata.insert(
            ANYCODE_RESPONSE_ID_METADATA_KEY.to_string(),
            chain_state_json("resp_123", &h),
        );
        // microcompact 式原地改写前缀
        history[0] = msg(MessageRole::User, "hi (stubbed)");
        assert_eq!(chain_plan(&history, true), ChainPlan::Full);
    }

    #[test]
    fn ephemeral_reminder_excluded_from_hash_but_in_tail() {
        let mut history = vec![
            msg(MessageRole::User, "hi"),
            msg(MessageRole::Assistant, "hello"),
            msg(MessageRole::User, "reminder"),
        ];
        history[2].metadata.insert(
            REPLY_LANGUAGE_REMINDER_METADATA_KEY.to_string(),
            json!(true),
        );
        let h = hash_prefix(&history[..1]);
        history[1].metadata.insert(
            ANYCODE_RESPONSE_ID_METADATA_KEY.to_string(),
            chain_state_json("resp_9", &h),
        );
        // ephemeral 在尾部：仍可链式，tail 含 reminder
        assert_eq!(
            chain_plan(&history, true),
            ChainPlan::Chained {
                previous_response_id: "resp_9".into(),
                tail_start: 2
            }
        );
    }

    #[test]
    fn is_chain_miss_matches_previous_response_errors() {
        assert!(is_chain_miss(
            400,
            r#"{"error":{"message":"previous_response_id resp_x not found"}}"#
        ));
        assert!(is_chain_miss(400, "previous response expired"));
        assert!(!is_chain_miss(404, "previous_response_id not found"));
        assert!(!is_chain_miss(400, "model not found"));
    }
}
