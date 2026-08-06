//! Structured live trace events for dashboard SSE (emit before disk log).

use serde::{Deserialize, Serialize};

/// Runtime → dashboard live trace (SSE-first; log is audit/replay).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LiveTraceEvent {
    TurnStart {
        turn: u32,
    },
    LlmRequestStart {
        turn: u32,
    },
    AssistantDelta {
        turn: u32,
        delta: String,
        /// When true, this turn's assistant text is intermediate tool-round narration.
        #[serde(default)]
        narration: bool,
    },
    /// Model reasoning / thinking chain (shown in tool-strip fold, not final reply).
    ThinkingDelta {
        turn: u32,
        delta: String,
    },
    /// Re-tag a turn's assistant block as tool-round narration (after tool_calls are known).
    AssistantNarrationMark {
        turn: u32,
    },
    /// User-facing progress card (intent / execute / discovery / deliver).
    ProgressUpdate {
        turn: u32,
        seq: u32,
        phase: String,
        work_stage: Option<String>,
        summary: String,
        next: Option<String>,
        discovery: Option<String>,
        evidence_refs: Vec<String>,
    },
    ToolCallStart {
        turn: u32,
        idx: u32,
        name: String,
        input_preview: String,
    },
    ToolCallEnd {
        turn: u32,
        idx: u32,
        name: String,
        elapsed_ms: u64,
        error: Option<String>,
        output_preview: String,
    },
    ToolCallProgress {
        turn: u32,
        idx: u32,
        name: String,
        elapsed_ms: u64,
    },
    AssistantDone {
        turn: u32,
        text: String,
    },
    TurnDone {
        status: String,
    },
    /// Structured file deliverable ready for conversation card + artifacts index.
    ArtifactReady {
        turn: u32,
        idx: u32,
        tool_name: String,
        artifact: crate::Artifact,
    },
    /// 嵌套子代理事件包装：内部事件按子代理身份打标后转发给父级。
    /// 包装把打标集中在一处（nested_task 的 forwarder），深度 >1 自动再包一层。
    Subagent {
        task_id: uuid::Uuid,
        agent_type: String,
        parent_task_id: Option<uuid::Uuid>,
        event: Box<LiveTraceEvent>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_event_serde_roundtrip_preserves_identity_and_inner() {
        let task_id = uuid::Uuid::new_v4();
        let parent_task_id = uuid::Uuid::new_v4();
        let evt = LiveTraceEvent::Subagent {
            task_id,
            agent_type: "explore".to_string(),
            parent_task_id: Some(parent_task_id),
            event: Box::new(LiveTraceEvent::ToolCallStart {
                turn: 2,
                idx: 1,
                name: "Grep".to_string(),
                input_preview: "pattern".to_string(),
            }),
        };
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: LiveTraceEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, evt);
        // 深度 >1：包装内再包装也能往返。
        let nested = LiveTraceEvent::Subagent {
            task_id: uuid::Uuid::new_v4(),
            agent_type: "general".to_string(),
            parent_task_id: Some(task_id),
            event: Box::new(back),
        };
        let json = serde_json::to_string(&nested).expect("serialize nested");
        let back: LiveTraceEvent = serde_json::from_str(&json).expect("deserialize nested");
        assert_eq!(back, nested);
    }

    #[test]
    fn subagent_event_without_parent_omits_none_cleanly() {
        let evt = LiveTraceEvent::Subagent {
            task_id: uuid::Uuid::new_v4(),
            agent_type: "plan".to_string(),
            parent_task_id: None,
            event: Box::new(LiveTraceEvent::TurnStart { turn: 1 }),
        };
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: LiveTraceEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, evt);
    }
}
