//! Map persisted project events / log lines into [`ChatStreamEvent`] for session SSE.

use crate::observability::log_parser::ParsedLine;
use crate::schema::{ChatStreamEvent, ProjectEvent, TranscriptBlock};
use anycode_core::strip_llm_reasoning_for_display;
use anycode_dashboard_ipc::question_ipc::PendingQuestionRecord;
use chrono::Utc;
use serde_json::{json, Value};

fn artifact_title_for_path_fallback(path: &str, kind: &str) -> String {
    let title = anycode_core::artifact_title_for_path(path);
    if title.is_empty() {
        kind.to_string()
    } else {
        title
    }
}

/// Strip `ANYCODE_ARTIFACT:{...}` protocol lines from assistant display text.
fn strip_artifact_markers(text: &str) -> String {
    let mut marker_paths = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(json_part) = trimmed.strip_prefix("ANYCODE_ARTIFACT:") else {
            continue;
        };
        if let Ok(v) = serde_json::from_str::<Value>(json_part.trim()) {
            if let Some(path) = v.get("path").and_then(|p| p.as_str()) {
                marker_paths.push(path.to_string());
            }
        }
    }
    let had_markers = !marker_paths.is_empty();
    let stripped = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.contains("ANYCODE_ARTIFACT:") {
                return false;
            }
            if marker_paths.iter().any(|path| path == trimmed) {
                return false;
            }
            if had_markers && is_artifact_scaffold_line(trimmed, &marker_paths) {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if is_artifact_scaffold_only(&stripped) {
        return String::new();
    }
    stripped
}

fn is_artifact_scaffold_line(line: &str, marker_paths: &[String]) -> bool {
    use std::path::Path;

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.eq_ignore_ascii_case("anycode") {
        return true;
    }
    for path in marker_paths {
        let base = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if !base.is_empty() && trimmed == base {
            return true;
        }
        let base_no_ext = base.rsplit_once('.').map(|(name, _)| name).unwrap_or(base);
        if !base_no_ext.is_empty() && trimmed.eq_ignore_ascii_case(base_no_ext) {
            return true;
        }
    }
    false
}

fn is_artifact_scaffold_only(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.eq_ignore_ascii_case("anycode") {
        return true;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".markdown")
        || lower.ends_with(".html")
        || lower.ends_with(".pdf")
        || lower.ends_with(".pptx")
        || lower.ends_with(".xlsx")
}

fn turn_from_payload(payload: &Value) -> Option<u32> {
    payload
        .get("turn")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            payload
                .get("turn")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
        })
}

fn tool_key_from_payload(payload: &Value, turn: Option<u32>) -> Option<String> {
    if let Some(k) = payload.get("tool_key").and_then(|v| v.as_str()) {
        if !k.is_empty() {
            return Some(k.to_string());
        }
    }
    let idx = payload.get("idx").and_then(|v| v.as_str())?;
    let turn = turn?;
    Some(format!("{turn}:{idx}"))
}

pub fn chat_event_from_project_event(evt: &ProjectEvent) -> Option<ChatStreamEvent> {
    let session_id = evt.session_id.as_deref()?.to_string();
    let turn = turn_from_payload(&evt.payload);
    let tool_key = tool_key_from_payload(&evt.payload, turn);
    let tool_name = evt
        .payload
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let kind = match evt.event_type.as_str() {
        "user_prompt" => "user_message",
        "tool_call_start" => "tool_start",
        "tool_call_end" => "tool_result",
        "assistant_response" => "assistant_done",
        "tool_approval_pending" => "approval_request",
        "tool_approval_resolved" => "approval_resolved",
        "ask_user_question_pending" => "question_request",
        "ask_user_question_resolved" => "question_resolved",
        "message_queued" => "message_queued",
        "message_dequeued" => "message_dequeued",
        "task_end" | "session_completed" => "turn_done",
        "session_error" | "session_blocked" | "session_cancelled" | "tool_denied" => {
            "session_error"
        }
        _ => return None,
    };

    let text = if evt.body.is_empty() {
        None
    } else {
        Some(evt.body.clone())
    };

    let block = match kind {
        "user_message" => Some(TranscriptBlock {
            id: evt.id.clone(),
            block_type: "user_message".into(),
            at: evt.occurred_at.clone(),
            title: evt.title.clone(),
            body: evt.body.clone(),
            meta: evt.payload.clone(),
            collapsible: false,
            default_collapsed: false,
            event_id: Some(evt.id.clone()),
        }),
        "assistant_done" => Some(TranscriptBlock {
            id: evt.id.clone(),
            block_type: "assistant_message".into(),
            at: evt.occurred_at.clone(),
            title: evt.title.clone(),
            body: evt.body.clone(),
            meta: evt.payload.clone(),
            collapsible: false,
            default_collapsed: false,
            event_id: Some(evt.id.clone()),
        }),
        "tool_start" => Some(TranscriptBlock {
            id: evt.id.clone(),
            block_type: "tool_call".into(),
            at: evt.occurred_at.clone(),
            title: evt.title.clone(),
            body: evt.body.clone(),
            meta: merge_tool_meta(&evt.payload, turn, tool_key.as_deref(), "start"),
            collapsible: true,
            default_collapsed: true,
            event_id: Some(evt.id.clone()),
        }),
        "tool_result" => Some(TranscriptBlock {
            id: evt.id.clone(),
            block_type: "tool_result".into(),
            at: evt.occurred_at.clone(),
            title: evt.title.clone(),
            body: evt.body.clone(),
            meta: merge_tool_meta(&evt.payload, turn, tool_key.as_deref(), "end"),
            collapsible: true,
            default_collapsed: true,
            event_id: Some(evt.id.clone()),
        }),
        "approval_request" => Some(TranscriptBlock {
            id: evt.id.clone(),
            block_type: "approval_request".into(),
            at: evt.occurred_at.clone(),
            title: evt.title.clone(),
            body: evt.body.clone(),
            meta: evt.payload.clone(),
            collapsible: true,
            default_collapsed: false,
            event_id: Some(evt.id.clone()),
        }),
        "approval_resolved" => {
            let mut meta = evt.payload.clone();
            if let Value::Object(ref mut map) = meta {
                map.insert("source".into(), json!("approval_resolved"));
                map.insert("severity".into(), json!("info"));
            }
            Some(TranscriptBlock {
                id: evt.id.clone(),
                block_type: "system_notice".into(),
                at: evt.occurred_at.clone(),
                title: evt.title.clone(),
                body: evt.body.clone(),
                meta,
                collapsible: true,
                default_collapsed: true,
                event_id: Some(evt.id.clone()),
            })
        }
        "question_request" => Some(TranscriptBlock {
            id: evt.id.clone(),
            block_type: "question_request".into(),
            at: evt.occurred_at.clone(),
            title: evt.title.clone(),
            body: evt.body.clone(),
            meta: evt.payload.clone(),
            collapsible: false,
            default_collapsed: false,
            event_id: Some(evt.id.clone()),
        }),
        "question_resolved" => {
            let mut meta = evt.payload.clone();
            if let Value::Object(ref mut map) = meta {
                map.insert("source".into(), json!("question_resolved"));
                map.insert("severity".into(), json!("info"));
            }
            Some(TranscriptBlock {
                id: evt.id.clone(),
                block_type: "system_notice".into(),
                at: evt.occurred_at.clone(),
                title: evt.title.clone(),
                body: evt.body.clone(),
                meta,
                collapsible: true,
                default_collapsed: true,
                event_id: Some(evt.id.clone()),
            })
        }
        "message_queued" => {
            let queue_id = evt
                .payload
                .get("queue_id")
                .and_then(|v| v.as_str())
                .unwrap_or(&evt.id);
            let mut meta = evt.payload.clone();
            if let Value::Object(ref mut map) = meta {
                map.insert("source".into(), json!("message_queue"));
                map.insert("status".into(), json!("pending"));
            }
            Some(TranscriptBlock {
                id: format!("queue:{queue_id}"),
                block_type: "user_message".into(),
                at: evt.occurred_at.clone(),
                title: evt.title.clone(),
                body: evt.body.clone(),
                meta,
                collapsible: false,
                default_collapsed: false,
                event_id: Some(evt.id.clone()),
            })
        }
        "message_dequeued" => None,
        _ => None,
    };

    Some(ChatStreamEvent {
        session_id,
        project_id: evt.project_id.clone(),
        kind: kind.into(),
        turn,
        conversation_turn_id: evt
            .payload
            .get("user_turn_id")
            .or_else(|| evt.payload.get("conversation_turn_id"))
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        seq: None,
        event_id: None,
        tool_key,
        tool_name,
        text,
        block,
        payload: evt.payload.clone(),
        at: evt.occurred_at.clone(),
    })
}

pub fn chat_event_from_parsed_line(
    session_id: &str,
    project_id: &str,
    parsed: &ParsedLine,
) -> Option<ChatStreamEvent> {
    if parsed.event_type == "assistant_response" {
        let turn = turn_from_payload(&parsed.payload).unwrap_or(1);
        if parsed.body.is_empty() {
            return None;
        }
        return Some(assistant_delta_event(
            session_id,
            project_id,
            0,
            turn,
            &parsed.body,
            &parsed.body,
            false,
        ));
    }

    let stable_id = stable_parsed_block_id(parsed);
    let evt = ProjectEvent {
        id: stable_id.clone(),
        project_id: project_id.to_string(),
        session_id: Some(session_id.to_string()),
        task_id: None,
        agent_id: None,
        event_type: parsed.event_type.clone(),
        severity: parsed.severity.clone(),
        title: parsed.title.clone(),
        body: parsed.body.clone(),
        payload: parsed.payload.clone(),
        occurred_at: Utc::now().to_rfc3339(),
    };
    let mut chat = chat_event_from_project_event(&evt)?;
    if let Some(block) = chat.block.as_mut() {
        block.id = stable_id;
        block.meta = merge_tool_meta(
            &block.meta,
            chat.turn,
            chat.tool_key.as_deref(),
            if chat.kind == "tool_result" {
                "end"
            } else {
                "start"
            },
        );
    }
    Some(chat)
}

/// Map in-process [`LiveTraceEvent`] → session `chat_event` SSE payload.
pub fn chat_event_from_live_trace(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    evt: &anycode_core::LiveTraceEvent,
    assistant_raw_buffers: &mut std::collections::HashMap<u32, String>,
    assistant_display_buffers: &mut std::collections::HashMap<u32, String>,
) -> Option<ChatStreamEvent> {
    let at = Utc::now().to_rfc3339();
    match evt {
        anycode_core::LiveTraceEvent::AssistantDelta {
            turn,
            delta,
            narration,
        } => {
            let raw = assistant_raw_buffers.entry(*turn).or_default();
            raw.push_str(delta);
            let new_display = strip_llm_reasoning_for_display(raw);
            let prev_display = assistant_display_buffers
                .get(turn)
                .cloned()
                .unwrap_or_default();
            let display_delta = display_text_suffix_delta(&prev_display, &new_display);
            assistant_display_buffers.insert(*turn, new_display.clone());
            if display_delta.is_empty() && new_display.is_empty() {
                return None;
            }
            Some(assistant_delta_event(
                session_id,
                project_id,
                user_turn_id,
                *turn,
                &display_delta,
                &new_display,
                *narration,
            ))
        }
        anycode_core::LiveTraceEvent::AssistantNarrationMark { turn } => {
            let new_display = assistant_display_buffers
                .get(turn)
                .cloned()
                .unwrap_or_default();
            if new_display.is_empty() {
                return None;
            }
            Some(assistant_delta_event(
                session_id,
                project_id,
                user_turn_id,
                *turn,
                "",
                &new_display,
                true,
            ))
        }
        anycode_core::LiveTraceEvent::ThinkingDelta { .. } => None,
        anycode_core::LiveTraceEvent::AssistantDone { turn, text } => {
            let display = strip_artifact_markers(&strip_llm_reasoning_for_display(text));
            assistant_raw_buffers.insert(*turn, text.clone());
            assistant_display_buffers.insert(*turn, display.clone());
            Some(ChatStreamEvent {
                session_id: session_id.to_string(),
                project_id: project_id.to_string(),
                kind: "assistant_done".into(),
                turn: Some(*turn),
                conversation_turn_id: Some(user_turn_id),
                seq: None,
                event_id: None,
                tool_key: None,
                tool_name: None,
                text: Some(display.clone()),
                block: Some(TranscriptBlock {
                    id: live_assistant_block_id(user_turn_id, *turn),
                    block_type: "assistant_message".into(),
                    at: at.clone(),
                    title: format!("Assistant (turn {turn})"),
                    body: display,
                    meta: live_assistant_meta(user_turn_id, *turn, false, false),
                    collapsible: false,
                    default_collapsed: false,
                    event_id: None,
                }),
                payload: json!({ "turn": turn, "user_turn_id": user_turn_id }),
                at,
            })
        }
        anycode_core::LiveTraceEvent::ToolCallStart {
            turn,
            idx,
            name,
            input_preview,
        } => {
            let tool_key = live_tool_key(user_turn_id, *turn, *idx);
            let id = live_tool_block_id(user_turn_id, *turn, *idx, "call");
            Some(ChatStreamEvent {
                session_id: session_id.to_string(),
                project_id: project_id.to_string(),
                kind: "tool_start".into(),
                turn: Some(*turn),
                conversation_turn_id: Some(user_turn_id),
                seq: None,
                event_id: None,
                tool_key: Some(tool_key.clone()),
                tool_name: Some(name.clone()),
                text: Some(input_preview.clone()),
                block: Some(TranscriptBlock {
                    id,
                    block_type: "tool_call".into(),
                    at: at.clone(),
                    title: format!("{name} started"),
                    body: input_preview.clone(),
                    meta: merge_tool_meta(
                        &json!({
                            "turn": turn.to_string(),
                            "idx": idx.to_string(),
                            "name": name,
                            "user_turn_id": user_turn_id.to_string(),
                        }),
                        Some(*turn),
                        Some(&tool_key),
                        "start",
                    ),
                    collapsible: true,
                    default_collapsed: true,
                    event_id: None,
                }),
                payload: json!({ "turn": turn, "idx": idx, "name": name, "user_turn_id": user_turn_id }),
                at,
            })
        }
        anycode_core::LiveTraceEvent::ToolCallProgress {
            turn,
            idx,
            name,
            elapsed_ms,
        } => {
            let tool_key = live_tool_key(user_turn_id, *turn, *idx);
            let id = live_tool_block_id(user_turn_id, *turn, *idx, "call");
            Some(ChatStreamEvent {
                session_id: session_id.to_string(),
                project_id: project_id.to_string(),
                kind: "tool_progress".into(),
                turn: Some(*turn),
                conversation_turn_id: Some(user_turn_id),
                seq: None,
                event_id: None,
                tool_key: Some(tool_key.clone()),
                tool_name: Some(name.clone()),
                text: None,
                block: Some(TranscriptBlock {
                    id,
                    block_type: "tool_call".into(),
                    at: at.clone(),
                    title: format!("{name} started"),
                    body: String::new(),
                    meta: merge_tool_meta(
                        &json!({
                            "turn": turn.to_string(),
                            "idx": idx.to_string(),
                            "name": name,
                            "elapsed_ms": elapsed_ms,
                            "duration_ms": elapsed_ms.to_string(),
                            "user_turn_id": user_turn_id.to_string(),
                        }),
                        Some(*turn),
                        Some(&tool_key),
                        "running",
                    ),
                    collapsible: true,
                    default_collapsed: true,
                    event_id: None,
                }),
                payload: json!({
                    "turn": turn,
                    "idx": idx,
                    "name": name,
                    "elapsed_ms": elapsed_ms,
                    "user_turn_id": user_turn_id,
                }),
                at,
            })
        }
        anycode_core::LiveTraceEvent::ToolCallEnd {
            turn,
            idx,
            name,
            elapsed_ms,
            error,
            output_preview,
        } => {
            let tool_key = live_tool_key(user_turn_id, *turn, *idx);
            let id = live_tool_block_id(user_turn_id, *turn, *idx, "result");
            let failed = error.is_some();
            let body = if failed {
                error.clone().unwrap_or_default()
            } else {
                output_preview.clone()
            };
            let mut end_meta = json!({
                "turn": turn.to_string(),
                "idx": idx.to_string(),
                "name": name,
                "elapsed_ms": elapsed_ms,
                "duration_ms": elapsed_ms.to_string(),
                "output_preview": output_preview,
                "user_turn_id": user_turn_id.to_string(),
            });
            if !failed {
                super::session_transcript::merge_activity_count_meta(&mut end_meta, name, &body);
            }
            Some(ChatStreamEvent {
                session_id: session_id.to_string(),
                project_id: project_id.to_string(),
                kind: "tool_result".into(),
                turn: Some(*turn),
                conversation_turn_id: Some(user_turn_id),
                seq: None,
                event_id: None,
                tool_key: Some(tool_key.clone()),
                tool_name: Some(name.clone()),
                text: if body.is_empty() {
                    None
                } else {
                    Some(body.clone())
                },
                block: Some(TranscriptBlock {
                    id,
                    block_type: "tool_result".into(),
                    at: at.clone(),
                    title: format!("{name} {}", if failed { "failed" } else { "finished" }),
                    body,
                    meta: merge_tool_meta(&end_meta, Some(*turn), Some(&tool_key), "end"),
                    collapsible: true,
                    default_collapsed: true,
                    event_id: None,
                }),
                payload: json!({ "turn": turn, "idx": idx, "name": name, "elapsed_ms": elapsed_ms, "user_turn_id": user_turn_id }),
                at,
            })
        }
        anycode_core::LiveTraceEvent::ProgressUpdate {
            turn,
            seq,
            phase,
            work_stage,
            summary,
            next,
            discovery,
            evidence_refs,
        } => Some(progress_update_event(
            session_id,
            project_id,
            user_turn_id,
            *turn,
            *seq,
            phase,
            work_stage.as_ref(),
            summary,
            next.as_ref(),
            discovery.as_ref(),
            evidence_refs,
            true,
            &at,
        )),
        anycode_core::LiveTraceEvent::TurnDone { status } => Some(ChatStreamEvent {
            session_id: session_id.to_string(),
            project_id: project_id.to_string(),
            kind: "turn_done".into(),
            turn: None,
            conversation_turn_id: Some(user_turn_id),
            seq: None,
            event_id: None,
            tool_key: None,
            tool_name: None,
            text: Some(status.clone()),
            block: None,
            payload: json!({ "status": status, "user_turn_id": user_turn_id }),
            at,
        }),
        anycode_core::LiveTraceEvent::ArtifactReady {
            turn,
            idx,
            tool_name,
            artifact,
        } => {
            let path = artifact.path.clone().unwrap_or_default();
            let kind = artifact.resolved_kind().to_string();
            let title = artifact
                .title
                .clone()
                .unwrap_or_else(|| artifact_title_for_path_fallback(&path, &kind));
            let id = format!(
                "deliverable:u{user_turn_id}:{turn}:{idx}:{}",
                path.replace('/', "_")
            );
            let mime = artifact
                .mime
                .clone()
                .unwrap_or_else(|| anycode_core::mime_for_path(&path).to_string());
            Some(ChatStreamEvent {
                session_id: session_id.to_string(),
                project_id: project_id.to_string(),
                kind: "deliverable".into(),
                turn: Some(*turn),
                conversation_turn_id: Some(user_turn_id),
                seq: None,
                event_id: None,
                tool_key: Some(live_tool_key(user_turn_id, *turn, *idx)),
                tool_name: Some(tool_name.clone()),
                text: Some(title.clone()),
                block: Some(TranscriptBlock {
                    id,
                    block_type: "deliverable".into(),
                    at: at.clone(),
                    title: title.clone(),
                    body: path.clone(),
                    meta: json!({
                        "path": path,
                        "kind": kind,
                        "mime": mime,
                        "title": title,
                        "project_id": project_id,
                        "preview_path": artifact.preview_path,
                        "bytes": artifact.bytes,
                        "inline": artifact.should_inline(),
                        "tool_name": tool_name,
                        "user_turn_id": user_turn_id.to_string(),
                        "turn": turn.to_string(),
                        "idx": idx.to_string(),
                    }),
                    collapsible: false,
                    default_collapsed: false,
                    event_id: None,
                }),
                payload: json!({
                    "artifact": artifact,
                    "user_turn_id": user_turn_id,
                }),
                at,
            })
        }
        anycode_core::LiveTraceEvent::TurnStart { .. } => None,
        anycode_core::LiveTraceEvent::LlmRequestStart { turn } => Some(ChatStreamEvent {
            session_id: session_id.to_string(),
            project_id: project_id.to_string(),
            kind: "llm_start".into(),
            turn: Some(*turn),
            conversation_turn_id: Some(user_turn_id),
            seq: None,
            event_id: None,
            tool_key: None,
            tool_name: None,
            text: None,
            block: Some(TranscriptBlock {
                id: format!("llm-start:u{user_turn_id}:{turn}"),
                block_type: "system_notice".into(),
                at: at.clone(),
                title: "Thinking".into(),
                body: String::new(),
                meta: json!({
                    "source": "llm_start",
                    "live": true,
                    "turn": turn,
                    "user_turn_id": user_turn_id.to_string(),
                }),
                collapsible: false,
                default_collapsed: false,
                event_id: None,
            }),
            payload: json!({ "turn": turn, "user_turn_id": user_turn_id }),
            at,
        }),
        // `Subagent` 包装由 bridge 解包后按 scope 递归映射（见
        // `apply_subagent_scope`）；直接到达这里说明调用方未解包，忽略。
        anycode_core::LiveTraceEvent::Subagent { .. } => None,
    }
}

/// 子代理作用域：嵌套任务事件映射到父时间线时携带的身份与命名空间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentScope {
    pub task_id: uuid::Uuid,
    pub agent_type: String,
    pub parent_task_id: Option<uuid::Uuid>,
}

impl SubagentScope {
    /// 写入 `payload["subagent"]` / `block.meta["subagent"]` 的身份标记。
    #[must_use]
    pub fn meta_json(&self) -> Value {
        json!({
            "task_id": self.task_id.to_string(),
            "agent_type": self.agent_type,
            "parent_task_id": self.parent_task_id.map(|id| id.to_string()),
        })
    }
}

/// 把已映射的 chat 事件按子代理身份命名空间化：key / 块 id 由
/// `u{user_turn_id}:{…}` 改写为 `u{user_turn_id}:sa{task_id}:{…}`，
/// 并在 payload 与 block.meta 打上 `subagent` 标记。
/// 无 scope 路径（`chat_event_from_live_trace` 直接返回）保持逐字节不变。
pub fn apply_subagent_scope(evt: &mut ChatStreamEvent, scope: &SubagentScope) {
    let user_turn_id = evt.conversation_turn_id.unwrap_or(0);
    let from = format!("u{user_turn_id}:");
    let to = format!("u{user_turn_id}:sa{}:", scope.task_id);
    if let Some(k) = evt.tool_key.as_mut() {
        *k = k.replacen(&from, &to, 1);
    }
    if let Some(block) = evt.block.as_mut() {
        block.id = block.id.replacen(&from, &to, 1);
        if let Value::Object(ref mut map) = block.meta {
            map.insert("subagent".into(), scope.meta_json());
        }
    }
    if let Value::Object(ref mut map) = evt.payload {
        map.insert("subagent".into(), scope.meta_json());
    }
}

/// 子代理组头：bridge 首次见到某 `task_id` 时发布（时间线可折叠组容器）。
pub fn subagent_header_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    scope: &SubagentScope,
) -> ChatStreamEvent {
    let at = Utc::now().to_rfc3339();
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "subagent_start".into(),
        turn: None,
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: Some(scope.agent_type.clone()),
        block: Some(TranscriptBlock {
            id: format!("subagent:u{user_turn_id}:sa{}", scope.task_id),
            block_type: "system_notice".into(),
            at: at.clone(),
            title: format!("Subagent {}", scope.agent_type),
            body: String::new(),
            meta: json!({
                "source": "subagent_start",
                "live": true,
                "user_turn_id": user_turn_id.to_string(),
                "subagent": scope.meta_json(),
            }),
            collapsible: true,
            default_collapsed: false,
            event_id: None,
        }),
        payload: json!({
            "user_turn_id": user_turn_id,
            "subagent": scope.meta_json(),
        }),
        at,
    }
}

/// 子代理组尾：内部 `TurnDone` 时发布（不冒泡为父级 `turn_done`，
/// 避免 UI 误判父轮次结束）。
pub fn subagent_done_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    scope: &SubagentScope,
    status: &str,
) -> ChatStreamEvent {
    let at = Utc::now().to_rfc3339();
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "subagent_done".into(),
        turn: None,
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: Some(status.to_string()),
        block: Some(TranscriptBlock {
            id: format!("subagent-done:u{user_turn_id}:sa{}", scope.task_id),
            block_type: "system_notice".into(),
            at: at.clone(),
            title: format!("Subagent {} {status}", scope.agent_type),
            body: String::new(),
            meta: json!({
                "source": "subagent_done",
                "status": status,
                "user_turn_id": user_turn_id.to_string(),
                "subagent": scope.meta_json(),
            }),
            collapsible: true,
            default_collapsed: true,
            event_id: None,
        }),
        payload: json!({
            "user_turn_id": user_turn_id,
            "status": status,
            "subagent": scope.meta_json(),
        }),
        at,
    }
}

pub fn turn_phase_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    turn: u32,
    phase: &str,
) -> ChatStreamEvent {
    let at = Utc::now().to_rfc3339();
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "turn_phase".into(),
        turn: Some(turn),
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: None,
        block: Some(TranscriptBlock {
            id: format!("turn-phase:u{user_turn_id}:{turn}"),
            block_type: "system_notice".into(),
            at: at.clone(),
            title: "Turn phase".into(),
            body: String::new(),
            meta: json!({
                "source": "turn_phase",
                "phase": phase,
                "live": true,
                "turn": turn,
                "user_turn_id": user_turn_id.to_string(),
                "started_at": at,
            }),
            collapsible: false,
            default_collapsed: false,
            event_id: None,
        }),
        payload: json!({ "phase": phase, "turn": turn, "user_turn_id": user_turn_id }),
        at,
    }
}

pub fn question_resolved_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    question_id: &str,
) -> ChatStreamEvent {
    let at = Utc::now().to_rfc3339();
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "question_resolved".into(),
        turn: None,
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: None,
        block: Some(TranscriptBlock {
            id: format!("question-resolved:{question_id}"),
            block_type: "system_notice".into(),
            at: at.clone(),
            title: "Question answered".into(),
            body: String::new(),
            meta: json!({
                "source": "question_resolved",
                "question_id": question_id,
                "user_turn_id": user_turn_id,
            }),
            collapsible: true,
            default_collapsed: true,
            event_id: None,
        }),
        payload: json!({ "question_id": question_id, "user_turn_id": user_turn_id }),
        at,
    }
}

pub fn approval_request_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    rec: &anycode_dashboard_ipc::approval_ipc::PendingApprovalRecord,
) -> ChatStreamEvent {
    let at = Utc::now().to_rfc3339();
    let payload = json!({
        "approval_id": rec.approval_id,
        "session_id": rec.session_id,
        "tool": rec.tool,
        "input_preview": rec.input_preview,
        "user_turn_id": user_turn_id,
    });
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "approval_request".into(),
        turn: None,
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: Some(rec.tool.clone()),
        text: Some(rec.input_preview.clone()),
        block: Some(TranscriptBlock {
            id: format!("approval-live:{}", rec.approval_id),
            block_type: "approval_request".into(),
            at: at.clone(),
            title: format!("Approve {}", rec.tool),
            body: rec.input_preview.clone(),
            meta: payload.clone(),
            collapsible: false,
            default_collapsed: false,
            event_id: None,
        }),
        payload,
        at,
    }
}

pub fn approval_resolved_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    approval_id: &str,
    decision: &str,
) -> ChatStreamEvent {
    let at = Utc::now().to_rfc3339();
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "approval_resolved".into(),
        turn: None,
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: Some(decision.to_string()),
        block: Some(TranscriptBlock {
            id: format!("approval-resolved:{approval_id}"),
            block_type: "system_notice".into(),
            at: at.clone(),
            title: "Approval resolved".into(),
            body: decision.to_string(),
            meta: json!({
                "source": "approval_resolved",
                "approval_id": approval_id,
                "decision": decision,
                "user_turn_id": user_turn_id,
            }),
            collapsible: true,
            default_collapsed: true,
            event_id: None,
        }),
        payload: json!({ "approval_id": approval_id, "decision": decision, "user_turn_id": user_turn_id }),
        at,
    }
}

pub fn question_request_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    rec: &PendingQuestionRecord,
) -> ChatStreamEvent {
    let at = Utc::now().to_rfc3339();
    let id = format!("question-live:{}", rec.question_id);
    let payload = json!({
        "question_id": rec.question_id,
        "session_id": rec.session_id,
        "header": rec.header,
        "options": rec.options,
        "multi_select": rec.multi_select,
        "user_turn_id": user_turn_id,
    });
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "question_request".into(),
        turn: None,
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: Some("AskUserQuestion".into()),
        text: Some(rec.question.clone()),
        block: Some(TranscriptBlock {
            id,
            block_type: "question_request".into(),
            at: at.clone(),
            title: rec.header.clone(),
            body: rec.question.clone(),
            meta: payload.clone(),
            collapsible: false,
            default_collapsed: false,
            event_id: None,
        }),
        payload,
        at,
    }
}

#[must_use]
pub fn live_tool_key(user_turn_id: u32, turn: u32, idx: u32) -> String {
    format!("u{user_turn_id}:{turn}:{idx}")
}

#[must_use]
pub fn live_tool_block_id(user_turn_id: u32, turn: u32, idx: u32, phase: &str) -> String {
    format!("tool-live:u{user_turn_id}:{turn}:{idx}:{phase}")
}

#[must_use]
pub fn live_progress_block_id(user_turn_id: u32, seq: u32) -> String {
    format!("progress-live:u{user_turn_id}:{seq}")
}

fn normalize_evidence_refs(user_turn_id: u32, refs: &[String]) -> Vec<String> {
    refs.iter()
        .map(|r| {
            if r.starts_with("tool:") {
                r.clone()
            } else if r.contains(':') {
                format!("tool:u{user_turn_id}:{r}")
            } else {
                r.clone()
            }
        })
        .collect()
}

fn progress_update_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    turn: u32,
    seq: u32,
    phase: &str,
    work_stage: Option<&String>,
    summary: &str,
    next: Option<&String>,
    discovery: Option<&String>,
    evidence_refs: &[String],
    live: bool,
    at: &str,
) -> ChatStreamEvent {
    let refs = normalize_evidence_refs(user_turn_id, evidence_refs);
    let body = summary.trim().to_string();
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "progress_update".into(),
        turn: Some(turn),
        conversation_turn_id: Some(user_turn_id),
        seq: Some(i64::from(seq)),
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: Some(body.clone()),
        block: Some(TranscriptBlock {
            id: live_progress_block_id(user_turn_id, seq),
            block_type: "progress_update".into(),
            at: at.to_string(),
            title: phase.to_string(),
            body,
            meta: json!({
                "live": live,
                "turn": turn,
                "seq": seq,
                "phase": phase,
                "work_stage": work_stage,
                "summary": summary,
                "next": next,
                "discovery": discovery,
                "evidence_refs": refs,
                "user_turn_id": user_turn_id.to_string(),
            }),
            collapsible: true,
            default_collapsed: !live,
            event_id: None,
        }),
        payload: json!({
            "turn": turn,
            "seq": seq,
            "phase": phase,
            "user_turn_id": user_turn_id,
        }),
        at: at.to_string(),
    }
}

#[must_use]
pub fn live_assistant_block_id(user_turn_id: u32, turn: u32) -> String {
    format!("assistant-live:u{user_turn_id}:{turn}")
}

fn live_assistant_meta(
    user_turn_id: u32,
    turn: u32,
    live: bool,
    narration: bool,
) -> serde_json::Value {
    let mut meta = serde_json::json!({
        "live": live,
        "turn": turn,
        "user_turn_id": user_turn_id.to_string(),
        "source": if live { serde_json::Value::String("llm_start".into()) } else { serde_json::Value::Null },
    });
    if narration {
        meta["narration"] = serde_json::json!(true);
        meta["message_role"] = serde_json::json!("status");
    }
    meta
}

pub fn thinking_delta_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    turn: u32,
    full_text: &str,
) -> ChatStreamEvent {
    let preview = full_text.trim();
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "thinking_delta".into(),
        turn: Some(turn),
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: Some(preview.to_string()),
        block: Some(TranscriptBlock {
            id: format!("thinking:u{user_turn_id}:{turn}"),
            block_type: "system_notice".into(),
            at: Utc::now().to_rfc3339(),
            title: "Thinking".into(),
            body: preview.to_string(),
            meta: json!({
                "source": "thinking_delta",
                "live": true,
                "turn": turn,
                "user_turn_id": user_turn_id.to_string(),
            }),
            collapsible: true,
            default_collapsed: true,
            event_id: None,
        }),
        payload: json!({ "turn": turn, "user_turn_id": user_turn_id }),
        at: Utc::now().to_rfc3339(),
    }
}

pub fn assistant_delta_event(
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    turn: u32,
    display_delta: &str,
    display_full: &str,
    narration: bool,
) -> ChatStreamEvent {
    ChatStreamEvent {
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        kind: "assistant_delta".into(),
        turn: Some(turn),
        conversation_turn_id: Some(user_turn_id),
        seq: None,
        event_id: None,
        tool_key: None,
        tool_name: None,
        text: Some(display_delta.to_string()),
        block: Some(TranscriptBlock {
            id: live_assistant_block_id(user_turn_id, turn),
            block_type: "assistant_message".into(),
            at: Utc::now().to_rfc3339(),
            title: format!("Assistant (turn {turn})"),
            body: display_full.to_string(),
            meta: live_assistant_meta(user_turn_id, turn, true, narration),
            collapsible: false,
            default_collapsed: false,
            event_id: None,
        }),
        payload: json!({
            "turn": turn,
            "delta": display_delta,
            "user_turn_id": user_turn_id
        }),
        at: Utc::now().to_rfc3339(),
    }
}

/// Incremental sanitized text: suffix of `new_display` after `prev_display`.
fn display_text_suffix_delta(prev_display: &str, new_display: &str) -> String {
    if new_display.starts_with(prev_display) {
        new_display[prev_display.len()..].to_string()
    } else if prev_display.is_empty() {
        new_display.to_string()
    } else {
        String::new()
    }
}

fn merge_tool_meta(
    payload: &Value,
    turn: Option<u32>,
    tool_key: Option<&str>,
    phase: &str,
) -> Value {
    let mut meta = payload.clone();
    if let Some(t) = turn {
        meta["turn"] = json!(t.to_string());
    }
    if let Some(k) = tool_key {
        meta["tool_key"] = json!(k);
    }
    meta["phase"] = json!(phase);
    meta
}

fn stable_parsed_block_id(parsed: &ParsedLine) -> String {
    match parsed.event_type.as_str() {
        "tool_call_start" | "tool_call_end" => {
            let turn = parsed
                .payload
                .get("turn")
                .and_then(|v| v.as_str())
                .unwrap_or("0");
            let idx = parsed
                .payload
                .get("idx")
                .and_then(|v| v.as_str())
                .unwrap_or("0");
            let phase = if parsed.event_type == "tool_call_end" {
                "result"
            } else {
                "call"
            };
            format!("tool-live:{turn}:{idx}:{phase}")
        }
        "assistant_response" => {
            let turn = parsed
                .payload
                .get("turn")
                .and_then(|v| v.as_str())
                .unwrap_or("1");
            format!("assistant-live:{turn}")
        }
        _ => uuid::Uuid::new_v4().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::log_parser::ParsedLine;

    #[test]
    fn strip_artifact_markers_removes_scaffold_echo() {
        let text = "anycode\n/tmp/mindmap-foo.md\nANYCODE_ARTIFACT:{\"path\":\"/tmp/mindmap-foo.md\",\"kind\":\"mindmap\",\"inline\":true}";
        assert_eq!(strip_artifact_markers(text), "");
        assert_eq!(strip_artifact_markers("anycode"), "");
    }

    #[test]
    fn maps_tool_start_from_project_event() {
        let evt = ProjectEvent {
            id: "e1".into(),
            project_id: "p1".into(),
            session_id: Some("s1".into()),
            task_id: None,
            agent_id: None,
            event_type: "tool_call_start".into(),
            severity: "info".into(),
            title: "Bash started".into(),
            body: "ls".into(),
            payload: json!({ "turn": "3", "idx": "1", "name": "Bash" }),
            occurred_at: "2026-01-01T00:00:00Z".into(),
        };
        let chat = chat_event_from_project_event(&evt).expect("mapped");
        assert_eq!(chat.kind, "tool_start");
        assert_eq!(chat.tool_key.as_deref(), Some("3:1"));
    }

    #[test]
    fn parsed_line_uses_stable_tool_ids() {
        let parsed = ParsedLine {
            event_type: "tool_call_start".into(),
            severity: "info".into(),
            title: "Bash started".into(),
            body: String::new(),
            payload: json!({ "turn": "2", "idx": "1", "name": "Bash" }),
        };
        let chat = chat_event_from_parsed_line("s1", "p1", &parsed).expect("mapped");
        assert_eq!(chat.kind, "tool_start");
        assert_eq!(
            chat.block.as_ref().map(|b| b.id.as_str()),
            Some("tool-live:2:1:call")
        );
    }

    #[test]
    fn live_trace_maps_tool_start() {
        let mut raw = std::collections::HashMap::new();
        let mut display = std::collections::HashMap::new();
        let chat = chat_event_from_live_trace(
            "s1",
            "p1",
            3,
            &anycode_core::LiveTraceEvent::ToolCallStart {
                turn: 2,
                idx: 1,
                name: "Bash".into(),
                input_preview: "ls".into(),
            },
            &mut raw,
            &mut display,
        )
        .expect("mapped");
        assert_eq!(chat.kind, "tool_start");
        assert_eq!(chat.tool_key.as_deref(), Some("u3:2:1"));
    }

    #[test]
    fn live_trace_assistant_delta_strips_redacted_thinking() {
        let mut raw = std::collections::HashMap::new();
        let mut display = std::collections::HashMap::new();
        let payload = [
            "<redacted",
            "_thinking>secret</redacted",
            "_thinking>\nHello",
        ]
        .concat();
        let chat = chat_event_from_live_trace(
            "s1",
            "p1",
            1,
            &anycode_core::LiveTraceEvent::AssistantDelta {
                turn: 1,
                delta: payload,
                narration: false,
            },
            &mut raw,
            &mut display,
        )
        .expect("visible tail");
        assert_eq!(chat.text.as_deref().map(str::trim), Some("Hello"));
        assert!(!chat.block.as_ref().unwrap().body.contains("secret"));
    }

    #[test]
    fn live_trace_narration_mark_tags_assistant_block() {
        let mut raw = std::collections::HashMap::new();
        let mut display = std::collections::HashMap::new();
        raw.insert(2, "Now let me check".into());
        display.insert(2, "Now let me check".into());
        let chat = chat_event_from_live_trace(
            "s1",
            "p1",
            1,
            &anycode_core::LiveTraceEvent::AssistantNarrationMark { turn: 2 },
            &mut raw,
            &mut display,
        )
        .expect("narration mark");
        assert_eq!(chat.kind, "assistant_delta");
        let meta = chat.block.as_ref().unwrap().meta.clone();
        assert_eq!(meta.get("narration").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            meta.get("message_role").and_then(|v| v.as_str()),
            Some("status")
        );
    }

    #[test]
    fn live_trace_progress_update_maps_to_block() {
        let mut raw = std::collections::HashMap::new();
        let mut display = std::collections::HashMap::new();
        let chat = chat_event_from_live_trace(
            "s1",
            "p1",
            2,
            &anycode_core::LiveTraceEvent::ProgressUpdate {
                turn: 3,
                seq: 1,
                phase: "execute".into(),
                work_stage: Some("inspect".into()),
                summary: "Checking tests".into(),
                next: Some("Run grep".into()),
                discovery: None,
                evidence_refs: vec!["3:1".into()],
            },
            &mut raw,
            &mut display,
        )
        .expect("progress");
        assert_eq!(chat.kind, "progress_update");
        let block = chat.block.expect("block");
        assert_eq!(block.block_type, "progress_update");
        assert_eq!(
            block.meta.get("phase").and_then(|v| v.as_str()),
            Some("execute")
        );
    }

    #[test]
    fn turn_phase_event_carries_phase_meta() {
        let evt = turn_phase_event("s1", "p1", 2, 3, "waiting_first_token");
        assert_eq!(evt.kind, "turn_phase");
        let block = evt.block.expect("block");
        assert_eq!(
            block.meta.get("source").and_then(|v| v.as_str()),
            Some("turn_phase")
        );
        assert_eq!(
            block.meta.get("phase").and_then(|v| v.as_str()),
            Some("waiting_first_token")
        );
    }

    fn map_tool_start_scoped(scope_task: uuid::Uuid) -> (ChatStreamEvent, SubagentScope) {
        let scope = SubagentScope {
            task_id: scope_task,
            agent_type: "explore".into(),
            parent_task_id: Some(uuid::Uuid::new_v4()),
        };
        let mut raw = std::collections::HashMap::new();
        let mut display = std::collections::HashMap::new();
        let mut chat = chat_event_from_live_trace(
            "s1",
            "p1",
            3,
            &anycode_core::LiveTraceEvent::ToolCallStart {
                turn: 2,
                idx: 1,
                name: "Grep".into(),
                input_preview: "needle".into(),
            },
            &mut raw,
            &mut display,
        )
        .expect("mapped");
        apply_subagent_scope(&mut chat, &scope);
        (chat, scope)
    }

    #[test]
    fn subagent_scope_namespaces_keys_and_tags_meta() {
        let tid = uuid::Uuid::new_v4();
        let (chat, scope) = map_tool_start_scoped(tid);
        assert_eq!(
            chat.tool_key.as_deref(),
            Some(format!("u3:sa{tid}:2:1").as_str())
        );
        let block = chat.block.as_ref().expect("block");
        assert_eq!(block.id, format!("tool-live:u3:sa{tid}:2:1:call"));
        assert_eq!(
            block.meta.get("subagent").and_then(|v| v.get("task_id")),
            Some(&json!(tid.to_string()))
        );
        assert_eq!(
            block.meta.get("subagent").and_then(|v| v.get("agent_type")),
            Some(&json!("explore"))
        );
        assert_eq!(chat.payload.get("subagent"), Some(&scope.meta_json()));
    }

    #[test]
    fn subagent_scope_keys_do_not_collide_across_children() {
        let (a, _) = map_tool_start_scoped(uuid::Uuid::new_v4());
        let (b, _) = map_tool_start_scoped(uuid::Uuid::new_v4());
        assert_ne!(a.tool_key, b.tool_key);
        assert_ne!(a.block.unwrap().id, b.block.unwrap().id);
    }

    #[test]
    fn live_trace_without_scope_stays_byte_identical() {
        // 无 scope 路径的形状钉死：subagent 改动不得影响既有事件映射。
        let mut raw = std::collections::HashMap::new();
        let mut display = std::collections::HashMap::new();
        let mut chat = chat_event_from_live_trace(
            "s1",
            "p1",
            3,
            &anycode_core::LiveTraceEvent::ToolCallStart {
                turn: 2,
                idx: 1,
                name: "Grep".into(),
                input_preview: "needle".into(),
            },
            &mut raw,
            &mut display,
        )
        .expect("mapped");
        chat.at = "<at>".into();
        if let Some(block) = chat.block.as_mut() {
            block.at = "<at>".into();
        }
        let value = serde_json::to_value(&chat).expect("serialize");
        let expected = json!({
            "session_id": "s1",
            "project_id": "p1",
            "kind": "tool_start",
            "turn": 2,
            "conversation_turn_id": 3,
            "tool_key": "u3:2:1",
            "tool_name": "Grep",
            "text": "needle",
            "block": {
                "id": "tool-live:u3:2:1:call",
                "block_type": "tool_call",
                "at": "<at>",
                "title": "Grep started",
                "body": "needle",
                "meta": {
                    "turn": "2",
                    "idx": "1",
                    "name": "Grep",
                    "user_turn_id": "3",
                    "tool_key": "u3:2:1",
                    "phase": "start",
                },
                "collapsible": true,
                "default_collapsed": true,
            },
            "payload": { "turn": 2, "idx": 1, "name": "Grep", "user_turn_id": 3 },
            "at": "<at>",
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn subagent_header_and_done_events_carry_scope_meta() {
        let scope = SubagentScope {
            task_id: uuid::Uuid::new_v4(),
            agent_type: "plan".into(),
            parent_task_id: None,
        };
        let header = subagent_header_event("s1", "p1", 3, &scope);
        assert_eq!(header.kind, "subagent_start");
        let block = header.block.as_ref().expect("block");
        assert_eq!(block.block_type, "system_notice");
        assert!(block.collapsible);
        assert!(!block.default_collapsed);
        assert_eq!(
            block.meta.get("subagent").and_then(|v| v.get("agent_type")),
            Some(&json!("plan"))
        );
        let done = subagent_done_event("s1", "p1", 3, &scope, "completed");
        assert_eq!(done.kind, "subagent_done");
        assert_eq!(
            done.payload.get("status").and_then(|v| v.as_str()),
            Some("completed")
        );
    }
}
