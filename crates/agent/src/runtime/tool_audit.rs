//! Best-effort tool governance audit log.
//!
//! This is intentionally independent from the approval path: failures to write the
//! audit log must never change tool behavior.

use anycode_core::prelude::*;
use chrono::Utc;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::fs::OpenOptions;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
struct ToolAuditRow<'a> {
    ts: String,
    task_id: String,
    phase: &'a str,
    tool_name: &'a str,
    working_directory: &'a str,
    input_hash: String,
    outcome: &'a str,
    detail: Option<&'a str>,
    call_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

fn audit_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".anycode/audit/tool-calls.jsonl"))
}

fn hash_value(v: &serde_json::Value) -> String {
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(v)
        .unwrap_or_else(|_| "<unserializable>".to_string())
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn append_tool_audit(
    task_id: TaskId,
    phase: &'static str,
    working_directory: &str,
    tool_call: &ToolCall,
    outcome: &'static str,
    detail: Option<&str>,
    duration_ms: Option<u64>,
) {
    let Some(path) = audit_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let row = ToolAuditRow {
        ts: Utc::now().to_rfc3339(),
        task_id: task_id.to_string(),
        phase,
        tool_name: &tool_call.name,
        working_directory,
        input_hash: hash_value(&tool_call.input),
        outcome,
        detail,
        call_id: &tool_call.id,
        duration_ms,
    };
    let Ok(line) = serde_json::to_string(&row) else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_value, ToolAuditRow};

    #[test]
    fn audit_hash_is_stable_for_same_json() {
        let a = serde_json::json!({"x": 1});
        assert_eq!(hash_value(&a), hash_value(&a));
    }

    #[test]
    fn result_row_serializes_duration_and_call_id() {
        let row = ToolAuditRow {
            ts: "2026-08-06T00:00:00Z".into(),
            task_id: "t1".into(),
            phase: "result",
            tool_name: "Bash",
            working_directory: "/tmp",
            input_hash: "abc".into(),
            outcome: "ok",
            detail: None,
            call_id: "call_123",
            duration_ms: Some(42),
        };
        let line = serde_json::to_string(&row).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&line).expect("parse");
        assert_eq!(v["duration_ms"], 42);
        assert_eq!(v["call_id"], "call_123");
    }

    #[test]
    fn execute_phase_row_omits_duration() {
        let row = ToolAuditRow {
            ts: "2026-08-06T00:00:00Z".into(),
            task_id: "t1".into(),
            phase: "execute",
            tool_name: "Bash",
            working_directory: "/tmp",
            input_hash: "abc".into(),
            outcome: "allowed",
            detail: None,
            call_id: "call_123",
            duration_ms: None,
        };
        let line = serde_json::to_string(&row).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&line).expect("parse");
        assert!(v.get("duration_ms").is_none());
    }
}
