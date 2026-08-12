//! Normalize LLM token usage from trace log lines into index-tier `llm_usage` events.

use crate::log_parser::ParsedLine;
use serde_json::{json, Value};

pub const EVENT_TYPE: &str = "llm_usage";

/// Build a normalized usage payload from a parsed `llm_response_end` line.
#[must_use]
pub fn usage_payload_from_parsed(parsed: &ParsedLine) -> Option<Value> {
    usage_payload_from_parsed_with_model(parsed, None)
}

/// Same as [`usage_payload_from_parsed`], optionally attaching a resolved model id.
#[must_use]
pub fn usage_payload_from_parsed_with_model(
    parsed: &ParsedLine,
    model: Option<&str>,
) -> Option<Value> {
    if parsed.event_type != "llm_response_end" {
        return None;
    }
    let turn = parsed
        .payload
        .get("turn")
        .and_then(|v| v.as_str().map(str::to_string))
        .or_else(|| {
            parsed
                .payload
                .get("turn")
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
        })
        .unwrap_or_else(|| "0".into());
    let input_tokens = token_int(&parsed.payload, "input_tokens");
    let output_tokens = token_int(&parsed.payload, "output_tokens");
    let elapsed_ms = token_int(&parsed.payload, "elapsed_ms");
    let cache_read_tokens = token_int(&parsed.payload, "cache_read_tokens");
    let cache_creation_tokens = token_int(&parsed.payload, "cache_creation_tokens");
    let model = model.map(str::trim).filter(|s| !s.is_empty()).or_else(|| {
        parsed
            .payload
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    });
    // `llm_response_end` 行由 execute_task 写入时自带 agent_type=（含嵌套子代理日志）；
    // 透传进 payload 解锁 by_agent 分组（execute_turn 路径无此字段，由 session join 兜底）。
    let agent_type = parsed
        .payload
        .get("agent_type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut payload = json!({
        "turn": turn,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "elapsed_ms": elapsed_ms,
        "cache_read_tokens": cache_read_tokens,
        "cache_creation_tokens": cache_creation_tokens,
    });
    if let Some(model) = model {
        payload["model"] = json!(model);
    }
    if let Some(agent_type) = agent_type {
        payload["agent_type"] = json!(agent_type);
    }
    Some(payload)
}

#[must_use]
pub fn usage_dedup_key(turn: &str) -> String {
    format!("{EVENT_TYPE}:turn:{turn}")
}

fn token_int(payload: &Value, key: &str) -> i64 {
    payload
        .get(key)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().map(|n| n as i64))
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_parser::parse_line;

    #[test]
    fn builds_usage_payload_from_llm_response_end() {
        let parsed = parse_line(
            "[llm_response_end] turn=2 elapsed_ms=1200 input_tokens=100 output_tokens=50",
        )
        .unwrap();
        let payload = usage_payload_from_parsed(&parsed).unwrap();
        assert_eq!(payload["turn"], "2");
        assert_eq!(payload["input_tokens"], 100);
        assert_eq!(payload["output_tokens"], 50);
        assert_eq!(payload["elapsed_ms"], 1200);
        assert!(payload.get("model").is_none());
    }

    #[test]
    fn builds_usage_payload_with_model() {
        let parsed =
            parse_line("[llm_response_end] turn=1 elapsed_ms=10 input_tokens=1 output_tokens=2")
                .unwrap();
        let payload =
            usage_payload_from_parsed_with_model(&parsed, Some("claude-sonnet-4")).unwrap();
        assert_eq!(payload["model"], "claude-sonnet-4");
    }

    #[test]
    fn passes_through_agent_type_when_present() {
        let parsed = parse_line(
            "[llm_response_end] turn=3 elapsed_ms=10 input_tokens=1 output_tokens=2 agent_type=explore",
        )
        .unwrap();
        let payload = usage_payload_from_parsed(&parsed).unwrap();
        assert_eq!(payload["agent_type"], "explore");
        assert_eq!(payload["turn"], "3");

        // 无 agent_type 字段（execute_turn 路径）→ payload 不带该键
        let parsed =
            parse_line("[llm_response_end] turn=1 elapsed_ms=10 input_tokens=1 output_tokens=2")
                .unwrap();
        let payload = usage_payload_from_parsed(&parsed).unwrap();
        assert!(payload.get("agent_type").is_none());
    }
}
