//! 与 Claude Code `microCompact.ts` 中非 cache-editing 路径对齐：在**完整压缩**前缩小摘要请求体积。
//!
//! anyCode 无 Anthropic cache_edits；采用与 **time-based microcompact** 相同的**内容清空**策略：
//! 对可压缩工具保留最近 `keep_recent` 条 tool_result，其余替换为占位文案。
//!
//! 额外（DeepSeek / OpenAI-compat 无 cache_edits）：
//! - 成功 FileWrite/Edit 后 stub `tool_calls[].input` 正文，避免整页 HTML 每 hop 回传；
//! - 旧 `reasoning_content` 只保留最近一轮 assistant（上游仍能 echo 当前链）。

use anycode_core::prelude::*;
use anycode_tools::catalog::{
    TOOL_BASH, TOOL_BROWSER_SCREENSHOT, TOOL_EDIT, TOOL_FILE_READ, TOOL_FILE_WRITE, TOOL_GLOB,
    TOOL_GREP, TOOL_NOTEBOOK_EDIT, TOOL_POWERSHELL, TOOL_SKILL, TOOL_TASK_OUTPUT, TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// 与 Claude `TIME_BASED_MC_CLEARED_MESSAGE` 一致（`microCompact.ts`）。
pub const CLEARED_TOOL_RESULT_PLACEHOLDER: &str = "[Old tool result content cleared]";

/// Stub marker so we do not re-hash already-stubbed write/edit args.
const STUBBED_ARGS_KEY: &str = "_anycode_stubbed";

fn is_compactable_tool(name: &str) -> bool {
    matches!(
        name,
        TOOL_FILE_READ
            | TOOL_BASH
            | TOOL_POWERSHELL
            | TOOL_GREP
            | TOOL_GLOB
            | TOOL_WEB_SEARCH
            | TOOL_WEB_FETCH
            | TOOL_EDIT
            | TOOL_FILE_WRITE
            | TOOL_NOTEBOOK_EDIT
            | TOOL_SKILL
            | TOOL_TASK_OUTPUT
            | TOOL_BROWSER_SCREENSHOT
    )
}

fn is_write_like_tool(name: &str) -> bool {
    matches!(name, TOOL_FILE_WRITE | TOOL_EDIT | TOOL_NOTEBOOK_EDIT)
}

fn collect_tool_use_id_to_name(msgs: &[Message]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for msg in msgs {
        if msg.role != MessageRole::Assistant {
            continue;
        }
        let Some(raw) = msg.metadata.get(ANYCODE_TOOL_CALLS_METADATA_KEY) else {
            continue;
        };
        let Ok(calls) = serde_json::from_value::<Vec<ToolCall>>(raw.clone()) else {
            continue;
        };
        for c in calls {
            m.insert(c.id.clone(), c.name.clone());
        }
    }
    m
}

/// 按对话顺序收集「可压缩工具」的 tool_use_id（仅含在 COMPACTABLE 集合内的调用）。
fn compactable_tool_use_ids_in_order(msgs: &[Message]) -> Vec<String> {
    let mut out = Vec::new();
    for msg in msgs {
        if msg.role != MessageRole::Assistant {
            continue;
        }
        let Some(raw) = msg.metadata.get(ANYCODE_TOOL_CALLS_METADATA_KEY) else {
            continue;
        };
        let Ok(calls) = serde_json::from_value::<Vec<ToolCall>>(raw.clone()) else {
            continue;
        };
        for c in calls {
            if is_compactable_tool(c.name.as_str()) {
                out.push(c.id.clone());
            }
        }
    }
    out
}

fn tool_name_for_result_msg(msg: &Message, id_to_name: &HashMap<String, String>) -> Option<String> {
    let MessageContent::ToolResult { tool_use_id, .. } = &msg.content else {
        return None;
    };
    if let Some(n) = id_to_name.get(tool_use_id) {
        return Some(n.clone());
    }
    msg.metadata
        .get("tool_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Successful (non-error) tool_use ids present in history.
fn successful_tool_use_ids(msgs: &[Message]) -> HashSet<String> {
    let mut ok = HashSet::new();
    for msg in msgs {
        if msg.role != MessageRole::Tool {
            continue;
        }
        let MessageContent::ToolResult {
            tool_use_id,
            is_error,
            ..
        } = &msg.content
        else {
            continue;
        };
        if !*is_error {
            ok.insert(tool_use_id.clone());
        }
    }
    ok
}

fn stub_write_edit_input(name: &str, input: &serde_json::Value) -> serde_json::Value {
    if input
        .get(STUBBED_ARGS_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return input.clone();
    }
    let path = input
        .get("file_path")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("notebook_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let body = if name == TOOL_FILE_WRITE {
        input
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .as_bytes()
    } else {
        // Edit / NotebookEdit: hash the replaced payload, not the whole file.
        let old = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Allocate once for hashing
        let combined = format!("{old}\n---\n{new}");
        return serde_json::json!({
            "file_path": path,
            "bytes": combined.len(),
            "sha256": hex::encode(Sha256::digest(combined.as_bytes())),
            STUBBED_ARGS_KEY: true,
            "note": "full edit args cleared after success; FileRead the path to revise"
        });
    };
    serde_json::json!({
        "file_path": path,
        "bytes": body.len(),
        "sha256": hex::encode(Sha256::digest(body)),
        STUBBED_ARGS_KEY: true,
        "note": "full write body cleared after success; FileRead the path to revise"
    })
}

/// After successful FileWrite/Edit, replace stored tool-call arguments with path+hash stub.
/// Keeps tool_use / tool_result pairing intact. Returns number of calls stubbed.
pub fn stub_successful_write_edit_args(messages: &mut [Message]) -> usize {
    let ok_ids = successful_tool_use_ids(messages);
    if ok_ids.is_empty() {
        return 0;
    }
    let mut stubbed = 0usize;
    for msg in messages.iter_mut() {
        if msg.role != MessageRole::Assistant {
            continue;
        }
        let Some(raw) = msg.metadata.get_mut(ANYCODE_TOOL_CALLS_METADATA_KEY) else {
            continue;
        };
        let Ok(mut calls) = serde_json::from_value::<Vec<ToolCall>>(raw.clone()) else {
            continue;
        };
        let mut changed = false;
        for c in &mut calls {
            if !is_write_like_tool(c.name.as_str()) {
                continue;
            }
            if !ok_ids.contains(&c.id) {
                continue;
            }
            if c.input
                .get(STUBBED_ARGS_KEY)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }
            // Skip tiny payloads (no token win).
            let approx = serde_json::to_string(&c.input)
                .map(|s| s.len())
                .unwrap_or(0);
            if approx < 512 {
                continue;
            }
            c.input = stub_write_edit_input(c.name.as_str(), &c.input);
            stubbed += 1;
            changed = true;
        }
        if changed {
            *raw = serde_json::to_value(&calls).unwrap_or_else(|_| raw.clone());
        }
    }
    stubbed
}

/// Keep `reasoning_content` only on the latest assistant message that has it.
/// Older thinking is unanchored for DeepSeek echo and burns tokens every hop.
pub fn strip_stale_reasoning_content(messages: &mut [Message]) -> usize {
    let mut last_with_rc: Option<usize> = None;
    for (i, msg) in messages.iter().enumerate() {
        if msg.role != MessageRole::Assistant {
            continue;
        }
        if msg
            .metadata
            .get(ANYCODE_REASONING_CONTENT_METADATA_KEY)
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty())
        {
            last_with_rc = Some(i);
        }
    }
    let Some(keep) = last_with_rc else {
        return 0;
    };
    let mut cleared = 0usize;
    for (i, msg) in messages.iter_mut().enumerate() {
        if i == keep || msg.role != MessageRole::Assistant {
            continue;
        }
        if msg
            .metadata
            .remove(ANYCODE_REASONING_CONTENT_METADATA_KEY)
            .is_some()
        {
            cleared += 1;
        }
    }
    cleared
}

/// Before each LLM hop: stub successful writes and drop stale thinking.
/// Does **not** clear tool_results (model may still need the latest turn).
pub fn prepare_messages_for_llm_hop(messages: &mut [Message]) -> LiveContextTrimStats {
    let stubbed_writes = stub_successful_write_edit_args(messages);
    let cleared_reasoning = strip_stale_reasoning_content(messages);
    LiveContextTrimStats {
        cleared_results: 0,
        stubbed_writes,
        cleared_reasoning,
    }
}

/// Live post-tool pass: microcompact results + stub writes + drop stale thinking.
pub fn apply_live_context_trim(messages: &mut [Message]) -> LiveContextTrimStats {
    let cleared_results = apply_microcompact_keep_latest_turn(messages);
    let stubbed_writes = stub_successful_write_edit_args(messages);
    let cleared_reasoning = strip_stale_reasoning_content(messages);
    LiveContextTrimStats {
        cleared_results,
        stubbed_writes,
        cleared_reasoning,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LiveContextTrimStats {
    pub cleared_results: usize,
    pub stubbed_writes: usize,
    pub cleared_reasoning: usize,
}

impl LiveContextTrimStats {
    pub fn any(&self) -> bool {
        self.cleared_results > 0 || self.stubbed_writes > 0 || self.cleared_reasoning > 0
    }
}

/// 清空较早的可压缩 tool_result，保留时间轴上最后 `keep_recent` 条（与 Claude `slice(-keepRecent)` 一致）。
/// 返回被替换的条数。
pub fn apply_microcompact(messages: &mut [Message], keep_recent: usize) -> usize {
    let ordered = compactable_tool_use_ids_in_order(messages);
    if ordered.is_empty() {
        return 0;
    }
    let keep_n = keep_recent.max(1);
    let start = ordered.len().saturating_sub(keep_n);
    let keep_set: HashSet<String> = ordered[start..].iter().cloned().collect();
    clear_compactable_except(messages, &keep_set)
}

/// Live 循环：保留**最近一条带 tool_calls 的 assistant** 对应的全部 tool_result，
/// 清空更早的可压缩结果。避免 `keep_recent=3` 把刚并行跑完、模型还没看见的结果清掉。
pub fn apply_microcompact_keep_latest_turn(messages: &mut [Message]) -> usize {
    let keep_ids = latest_turn_tool_use_ids(messages);
    if keep_ids.is_empty() {
        return 0;
    }
    clear_compactable_except(messages, &keep_ids)
}

fn latest_turn_tool_use_ids(msgs: &[Message]) -> HashSet<String> {
    for msg in msgs.iter().rev() {
        if msg.role != MessageRole::Assistant {
            continue;
        }
        let Some(raw) = msg.metadata.get(ANYCODE_TOOL_CALLS_METADATA_KEY) else {
            continue;
        };
        let Ok(calls) = serde_json::from_value::<Vec<ToolCall>>(raw.clone()) else {
            continue;
        };
        if calls.is_empty() {
            continue;
        }
        return calls.into_iter().map(|c| c.id).collect();
    }
    HashSet::new()
}

fn clear_compactable_except(messages: &mut [Message], keep_ids: &HashSet<String>) -> usize {
    let id_to_name = collect_tool_use_id_to_name(messages);
    let mut cleared = 0usize;
    for msg in messages.iter_mut() {
        if msg.role != MessageRole::Tool {
            continue;
        }
        let Some(name) = tool_name_for_result_msg(msg, &id_to_name) else {
            continue;
        };
        if !is_compactable_tool(name.as_str()) {
            continue;
        }
        let MessageContent::ToolResult {
            tool_use_id,
            content,
            ..
        } = &mut msg.content
        else {
            continue;
        };
        if keep_ids.contains(tool_use_id) {
            continue;
        }
        if content == CLEARED_TOOL_RESULT_PLACEHOLDER {
            continue;
        }
        *content = CLEARED_TOOL_RESULT_PLACEHOLDER.to_string();
        cleared += 1;
    }
    cleared
}

/// 默认保留条数（对齐 Claude time-based 配置常见 `keepRecent`）。
pub fn default_keep_recent() -> usize {
    std::env::var("ANYCODE_MICROCOMPACT_KEEP_RECENT")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn asst_with_tools(calls: Vec<ToolCall>) -> Message {
        let mut meta = HashMap::new();
        meta.insert(
            ANYCODE_TOOL_CALLS_METADATA_KEY.to_string(),
            serde_json::to_value(&calls).unwrap(),
        );
        Message {
            id: Uuid::new_v4(),
            role: MessageRole::Assistant,
            content: MessageContent::Text("ok".into()),
            timestamp: chrono::Utc::now(),
            metadata: meta,
        }
    }

    fn tool_res(id: &str, name: &str, body: &str) -> Message {
        let mut meta = HashMap::new();
        meta.insert(
            "tool_name".to_string(),
            serde_json::Value::String(name.to_string()),
        );
        Message {
            id: Uuid::new_v4(),
            role: MessageRole::Tool,
            content: MessageContent::ToolResult {
                tool_use_id: id.to_string(),
                content: body.to_string(),
                is_error: false,
            },
            timestamp: chrono::Utc::now(),
            metadata: meta,
        }
    }

    #[test]
    fn clears_old_keeps_last_two() {
        let mut msgs = vec![
            asst_with_tools(vec![
                ToolCall {
                    id: "a".into(),
                    name: TOOL_BASH.into(),
                    input: serde_json::json!({}),
                },
                ToolCall {
                    id: "b".into(),
                    name: TOOL_BASH.into(),
                    input: serde_json::json!({}),
                },
                ToolCall {
                    id: "c".into(),
                    name: TOOL_BASH.into(),
                    input: serde_json::json!({}),
                },
            ]),
            tool_res("a", TOOL_BASH, "out1"),
            tool_res("b", TOOL_BASH, "out2"),
            tool_res("c", TOOL_BASH, "out3"),
        ];
        let n = apply_microcompact(&mut msgs, 2);
        assert_eq!(n, 1);
        let c = |id: &str| {
            msgs.iter()
                .find_map(|m| match &m.content {
                    MessageContent::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } if tool_use_id == id => Some(content.as_str()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(c("a"), CLEARED_TOOL_RESULT_PLACEHOLDER);
        assert_eq!(c("b"), "out2");
        assert_eq!(c("c"), "out3");
    }

    #[test]
    fn keep_latest_turn_preserves_full_parallel_batch() {
        let mut msgs = vec![
            asst_with_tools(vec![ToolCall {
                id: "old".into(),
                name: TOOL_FILE_READ.into(),
                input: serde_json::json!({}),
            }]),
            tool_res("old", TOOL_FILE_READ, "old body"),
            asst_with_tools(
                (0..5)
                    .map(|i| ToolCall {
                        id: format!("n{i}"),
                        name: TOOL_FILE_READ.into(),
                        input: serde_json::json!({}),
                    })
                    .collect(),
            ),
        ];
        for i in 0..5 {
            msgs.push(tool_res(
                &format!("n{i}"),
                TOOL_FILE_READ,
                &format!("body{i}"),
            ));
        }
        let n = apply_microcompact_keep_latest_turn(&mut msgs);
        assert_eq!(n, 1);
        let c = |id: &str| {
            msgs.iter()
                .find_map(|m| match &m.content {
                    MessageContent::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } if tool_use_id == id => Some(content.as_str()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(c("old"), CLEARED_TOOL_RESULT_PLACEHOLDER);
        for i in 0..5 {
            assert_eq!(c(&format!("n{i}")), format!("body{i}").as_str());
        }
    }

    #[test]
    fn keep_latest_turn_noop_without_tool_calls() {
        let mut msgs = vec![tool_res("x", TOOL_BASH, "out")];
        assert_eq!(apply_microcompact_keep_latest_turn(&mut msgs), 0);
    }

    #[test]
    fn skill_results_are_compactable() {
        let mut msgs = vec![
            asst_with_tools(vec![ToolCall {
                id: "s1".into(),
                name: TOOL_SKILL.into(),
                input: serde_json::json!({"name": "anycode-ppt"}),
            }]),
            tool_res("s1", TOOL_SKILL, &"x".repeat(4000)),
            asst_with_tools(vec![ToolCall {
                id: "s2".into(),
                name: TOOL_SKILL.into(),
                input: serde_json::json!({"name": "anycode-ppt"}),
            }]),
            tool_res("s2", TOOL_SKILL, "latest skill body"),
        ];
        let n = apply_microcompact_keep_latest_turn(&mut msgs);
        assert_eq!(n, 1);
        let c = |id: &str| {
            msgs.iter()
                .find_map(|m| match &m.content {
                    MessageContent::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } if tool_use_id == id => Some(content.as_str()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(c("s1"), CLEARED_TOOL_RESULT_PLACEHOLDER);
        assert_eq!(c("s2"), "latest skill body");
    }

    #[test]
    fn stubs_large_filewrite_args_after_success() {
        let html = format!("<!DOCTYPE html>{}", "a".repeat(800));
        let mut msgs = vec![
            asst_with_tools(vec![ToolCall {
                id: "w1".into(),
                name: TOOL_FILE_WRITE.into(),
                input: serde_json::json!({
                    "file_path": "/tmp/slides/01.html",
                    "content": html,
                }),
            }]),
            tool_res(
                "w1",
                TOOL_FILE_WRITE,
                r#"{"success":true,"path":"/tmp/slides/01.html"}"#,
            ),
        ];
        let n = stub_successful_write_edit_args(&mut msgs);
        assert_eq!(n, 1);
        let raw = msgs[0]
            .metadata
            .get(ANYCODE_TOOL_CALLS_METADATA_KEY)
            .unwrap();
        let calls: Vec<ToolCall> = serde_json::from_value(raw.clone()).unwrap();
        let args = &calls[0].input;
        assert!(args.get("content").is_none());
        assert_eq!(args["file_path"], "/tmp/slides/01.html");
        assert!(args["_anycode_stubbed"].as_bool().unwrap());
        assert!(args["sha256"].as_str().unwrap().len() == 64);
        // Idempotent
        assert_eq!(stub_successful_write_edit_args(&mut msgs), 0);
    }

    #[test]
    fn strips_old_reasoning_keeps_latest() {
        let mut m1 = asst_with_tools(vec![]);
        m1.metadata.insert(
            ANYCODE_REASONING_CONTENT_METADATA_KEY.to_string(),
            serde_json::Value::String("old think".into()),
        );
        let mut m2 = asst_with_tools(vec![]);
        m2.metadata.insert(
            ANYCODE_REASONING_CONTENT_METADATA_KEY.to_string(),
            serde_json::Value::String("new think".into()),
        );
        let mut msgs = vec![m1, m2];
        assert_eq!(strip_stale_reasoning_content(&mut msgs), 1);
        assert!(msgs[0]
            .metadata
            .get(ANYCODE_REASONING_CONTENT_METADATA_KEY)
            .is_none());
        assert_eq!(
            msgs[1]
                .metadata
                .get(ANYCODE_REASONING_CONTENT_METADATA_KEY)
                .and_then(|v| v.as_str()),
            Some("new think")
        );
    }
}
