//! OpenAI Responses API 流式 SSE：语义化事件序列（`response.*`），
//! 以 `response.completed` / `response.incomplete` / `response.failed` 结束，
//! **没有** `data: [DONE]`（DeepSeek 文档明确）。

use anycode_core::prelude::*;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default)]
struct FnCallAcc {
    call_id: String,
    name: String,
    arguments: String,
}

/// 将单条 `data:` JSON（Responses 语义事件）解析为 0..n 个 [`StreamEvent`]。
pub struct ResponsesSseStreamState {
    fn_calls: HashMap<usize, FnCallAcc>,
    response_id: Option<String>,
    /// 链式 prefix hash（空串 = 端点不支持链式，不发 `ResponseId`）。
    prefix_hash: String,
    /// 是否已收到终止事件（completed / incomplete）；failed 走 [`StreamEvent::Failed`]。
    saw_terminal: bool,
}

impl ResponsesSseStreamState {
    pub fn new(prefix_hash: String) -> Self {
        Self {
            fn_calls: HashMap::new(),
            response_id: None,
            prefix_hash,
            saw_terminal: false,
        }
    }

    /// 流是否正常走完（收到 `response.completed` 或 `response.incomplete`）。
    pub fn is_complete(&self) -> bool {
        self.saw_terminal
    }

    pub fn push_json_str(&mut self, data: &str) -> Result<Vec<StreamEvent>, serde_json::Error> {
        let v: Value = serde_json::from_str(data)?;
        Ok(self.push_value(&v))
    }

    fn output_index(value: &Value) -> usize {
        value
            .get("output_index")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as usize
    }

    fn flush_fn_call(out: &mut Vec<StreamEvent>, mut acc: FnCallAcc, arguments: Option<&str>) {
        let raw = arguments.unwrap_or(&acc.arguments);
        let input: Value = if raw.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
        };
        if !acc.call_id.is_empty() && !acc.name.is_empty() {
            out.push(StreamEvent::ToolCall(ToolCall {
                id: std::mem::take(&mut acc.call_id),
                name: std::mem::take(&mut acc.name),
                input,
            }));
        }
    }

    fn push_value(&mut self, value: &Value) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        let Some(ev_type) = value.get("type").and_then(|t| t.as_str()) else {
            return out;
        };

        match ev_type {
            "response.created" => {
                if let Some(id) = value
                    .get("response")
                    .and_then(|r| r.get("id"))
                    .and_then(|x| x.as_str())
                {
                    self.response_id = Some(id.to_string());
                }
            }
            "response.output_text.delta" => {
                if let Some(d) = value.get("delta").and_then(|x| x.as_str()) {
                    if !d.is_empty() {
                        out.push(StreamEvent::Delta(d.to_string()));
                    }
                }
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                if let Some(d) = value.get("delta").and_then(|x| x.as_str()) {
                    if !d.is_empty() {
                        out.push(StreamEvent::Reasoning(d.to_string()));
                    }
                }
            }
            "response.output_item.added" => {
                let item = value.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    let idx = Self::output_index(value);
                    self.fn_calls.insert(
                        idx,
                        FnCallAcc {
                            call_id: item
                                .get("call_id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string(),
                            name: item
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string(),
                            arguments: String::new(),
                        },
                    );
                }
            }
            "response.function_call_arguments.delta" => {
                let idx = Self::output_index(value);
                if let Some(d) = value.get("delta").and_then(|x| x.as_str()) {
                    if let Some(acc) = self.fn_calls.get_mut(&idx) {
                        acc.arguments.push_str(d);
                    }
                }
            }
            "response.function_call_arguments.done" => {
                let idx = Self::output_index(value);
                let done_args = value.get("arguments").and_then(|x| x.as_str());
                if let Some(acc) = self.fn_calls.remove(&idx) {
                    Self::flush_fn_call(&mut out, acc, done_args);
                }
            }
            "response.output_item.done" => {
                let item = value.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    let idx = Self::output_index(value);
                    // arguments.done 已 flush 的不再重复；否则从完整 item 兜底
                    if let Some(acc) = self.fn_calls.remove(&idx) {
                        let mut acc = acc;
                        if acc.call_id.is_empty() {
                            acc.call_id = item
                                .get("call_id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                        }
                        if acc.name.is_empty() {
                            acc.name = item
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                        }
                        Self::flush_fn_call(
                            &mut out,
                            acc,
                            item.get("arguments").and_then(|x| x.as_str()),
                        );
                    }
                }
            }
            "response.completed" | "response.incomplete" => {
                self.saw_terminal = true;
                if let Some(id) = value
                    .get("response")
                    .and_then(|r| r.get("id"))
                    .and_then(|x| x.as_str())
                {
                    self.response_id = Some(id.to_string());
                }
                if let Some(usage) = value.get("response").and_then(|r| r.get("usage")) {
                    out.push(StreamEvent::Usage(Usage {
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
                    }));
                }
                if !self.prefix_hash.is_empty() {
                    if let Some(id) = &self.response_id {
                        out.push(StreamEvent::ResponseId {
                            id: id.clone(),
                            prefix_hash: self.prefix_hash.clone(),
                        });
                    }
                }
            }
            "response.failed" | "error" => {
                let msg = value
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|x| x.as_str())
                    .or_else(|| {
                        value
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|x| x.as_str())
                    })
                    .or_else(|| value.get("message").and_then(|x| x.as_str()))
                    .unwrap_or("response.failed");
                out.push(StreamEvent::Failed(msg.to_string()));
            }
            _ => {}
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_marks_terminal_and_emits_usage_and_response_id() {
        let mut s = ResponsesSseStreamState::new("abc123".to_string());
        s.push_value(&serde_json::json!({
            "type": "response.created",
            "response": { "id": "resp_1", "status": "in_progress" }
        }));
        assert!(!s.is_complete());
        let r = s.push_value(&serde_json::json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 20,
                    "input_tokens_details": { "cached_tokens": 64 }
                }
            }
        }));
        assert!(s.is_complete());
        assert!(matches!(
            &r[..],
            [StreamEvent::Usage(Usage {
                input_tokens: 100,
                output_tokens: 20,
                cache_read_tokens: Some(64),
                ..
            }), StreamEvent::ResponseId { id, prefix_hash }]
            if id == "resp_1" && prefix_hash == "abc123"
        ));
    }

    #[test]
    fn response_id_event_skipped_when_chaining_disabled() {
        let mut s = ResponsesSseStreamState::new(String::new());
        let r = s.push_value(&serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp_1", "usage": { "input_tokens": 1, "output_tokens": 1 } }
        }));
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0], StreamEvent::Usage(_)));
    }

    #[test]
    fn text_and_reasoning_deltas_emit() {
        let mut s = ResponsesSseStreamState::new(String::new());
        let r = s.push_value(&serde_json::json!({
            "type": "response.output_text.delta", "delta": "Hi"
        }));
        assert!(matches!(&r[..], [StreamEvent::Delta(t)] if t == "Hi"));
        let r = s.push_value(&serde_json::json!({
            "type": "response.reasoning_text.delta", "delta": "think"
        }));
        assert!(matches!(&r[..], [StreamEvent::Reasoning(t)] if t == "think"));
    }

    #[test]
    fn function_call_accumulates_and_flushes_once() {
        let mut s = ResponsesSseStreamState::new(String::new());
        s.push_value(&serde_json::json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": { "type": "function_call", "call_id": "call_9", "name": "Read" }
        }));
        s.push_value(&serde_json::json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "delta": "{\"path\":\"a"
        }));
        s.push_value(&serde_json::json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "delta": ".rs\"}"
        }));
        let r = s.push_value(&serde_json::json!({
            "type": "response.function_call_arguments.done",
            "output_index": 1,
            "arguments": "{\"path\":\"a.rs\"}"
        }));
        assert_eq!(r.len(), 1);
        match &r[0] {
            StreamEvent::ToolCall(tc) => {
                assert_eq!(tc.id, "call_9");
                assert_eq!(tc.name, "Read");
                assert_eq!(tc.input, serde_json::json!({"path": "a.rs"}));
            }
            _ => panic!("expected ToolCall"),
        }
        // output_item.done 不重复 flush
        let r = s.push_value(&serde_json::json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": { "type": "function_call", "call_id": "call_9", "name": "Read", "arguments": "{\"path\":\"a.rs\"}" }
        }));
        assert!(r.is_empty());
    }

    #[test]
    fn function_call_fallback_flush_from_output_item_done() {
        let mut s = ResponsesSseStreamState::new(String::new());
        s.push_value(&serde_json::json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": { "type": "function_call", "call_id": "c1", "name": "Echo" }
        }));
        let r = s.push_value(&serde_json::json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": { "type": "function_call", "call_id": "c1", "name": "Echo", "arguments": "" }
        }));
        assert!(matches!(&r[..], [StreamEvent::ToolCall(tc)] if tc.input == serde_json::json!({})));
    }

    #[test]
    fn failed_event_emits_failed() {
        let mut s = ResponsesSseStreamState::new(String::new());
        let r = s.push_value(&serde_json::json!({
            "type": "response.failed",
            "response": { "error": { "message": "boom" } }
        }));
        assert!(matches!(&r[..], [StreamEvent::Failed(m)] if m == "boom"));
        assert!(!s.is_complete());
    }
}
