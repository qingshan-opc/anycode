//! LLM-backed verification gates: run an agent profile as a gate.
//!
//! Wire 兼容：handler 看到 `llm:` 前缀命令就走这里而非 shell；产出的
//! `GateExecuteResult` 与 shell 门禁同形，`persist_manual_gate_run` 与信任流零改动。
//! 裁决语义 default-to-refuted：缺 structured output / schema 非法 / verdict != "pass"
//! 一律 `failed`。

use super::chat_runtime::ChatRuntimeHost;
use super::gate_runner::{critic_gate_schema, GateExecuteResult, LLM_GATE_COMMAND_PREFIX};
use anycode_core::{AgentType, Message, MessageContent, MessageRole, TaskBudget};
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use uuid::Uuid;

const CRITIC_GATE_NAME: &str = "critic review (LLM)";

pub async fn execute_critic_gate(
    host: &ChatRuntimeHost,
    project_root: &Path,
) -> Result<GateExecuteResult> {
    let t0 = Instant::now();
    let runtime = host.runtime_for_gate(project_root).await?;
    let wd = project_root.to_string_lossy().to_string();
    let agent_type = AgentType::new("critic");
    let task_id = Uuid::new_v4();

    let services = runtime
        .tool_services()
        .context("embedded runtime has no tool services")?;
    let schema = critic_gate_schema();
    services.set_structured_output_schema(task_id, schema.clone());

    let mut messages = runtime
        .build_session_messages(&agent_type, &wd)
        .await
        .context("build critic session messages")?;
    messages.push(Message {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        content: MessageContent::Text(format!(
            "Adversarially verify the project at {wd}. Recent work claims to be complete and \
             correct — assume it is not. Inspect recent changes (git status/diff if this is a \
             repo), run the relevant test/build commands yourself via Bash, and look for \
             untested paths, stale artifacts, and missing edge cases.\n\
             {}{}",
            anycode_tools::STRUCTURED_OUTPUT_INSTRUCTION,
            schema
        )),
        timestamp: chrono::Utc::now(),
        metadata: Default::default(),
    });

    let messages = Arc::new(Mutex::new(messages));
    let deny = vec!["Edit".to_string(), "FileWrite".to_string()];
    let turn = runtime
        .execute_turn_from_messages(
            task_id,
            &agent_type,
            Arc::clone(&messages),
            &wd,
            None,
            &deny,
            &[],
            TaskBudget::default(),
            super::chat_runtime::embedded_loop_limits(),
            None,
        )
        .await;

    let structured = services.take_structured_output(task_id);
    let turn_error = turn.err().map(|e| e.to_string());
    Ok(gate_result_from_structured(
        structured,
        turn_error,
        t0.elapsed().as_millis() as u64,
    ))
}

/// 纯映射（单测友好）：structured output + turn 错误 → 门禁结果。
pub(crate) fn gate_result_from_structured(
    structured: Option<serde_json::Value>,
    turn_error: Option<String>,
    elapsed_ms: u64,
) -> GateExecuteResult {
    let result = |status: &str, excerpt: String| GateExecuteResult {
        name: CRITIC_GATE_NAME.into(),
        command: format!("{LLM_GATE_COMMAND_PREFIX}critic"),
        status: status.into(),
        output_excerpt: super::gate_runner::truncate_gate_output(&excerpt, 4000),
        elapsed_ms,
    };
    if let Some(e) = turn_error {
        return result("failed", format!("critic turn error: {e}"));
    }
    let Some(v) = structured else {
        return result(
            "failed",
            "critic produced no structured verdict; default verdict is REFUTED".into(),
        );
    };
    let verdict = v.get("verdict").and_then(|x| x.as_str()).unwrap_or("");
    let findings = render_findings(&v);
    if verdict == "pass" {
        let excerpt = if findings.is_empty() {
            "critic: pass (adversarial checks found no surviving refutation)".into()
        } else {
            format!("critic: pass with notes\n{findings}")
        };
        result("passed", excerpt)
    } else {
        let excerpt = if findings.is_empty() {
            format!("critic verdict: {verdict} (no findings detail)")
        } else {
            format!("critic verdict: {verdict}\n{findings}")
        };
        result("failed", excerpt)
    }
}

fn render_findings(v: &serde_json::Value) -> String {
    let Some(arr) = v.get("findings").and_then(|f| f.as_array()) else {
        return String::new();
    };
    arr.iter()
        .take(20)
        .map(|f| {
            let sev = f.get("severity").and_then(|s| s.as_str()).unwrap_or("info");
            let detail = f.get("detail").and_then(|s| s.as_str()).unwrap_or("");
            format!("- [{sev}] {detail}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_verdict_maps_to_passed() {
        let v = serde_json::json!({"verdict": "pass", "findings": []});
        let r = gate_result_from_structured(Some(v), None, 10);
        assert_eq!(r.status, "passed");
        assert_eq!(r.command, "llm:critic");
    }

    #[test]
    fn fail_verdict_maps_to_failed_with_findings() {
        let v = serde_json::json!({
            "verdict": "fail",
            "findings": [{"severity": "high", "detail": "src/main.rs:12 unwrap panics on empty input"}],
        });
        let r = gate_result_from_structured(Some(v), None, 10);
        assert_eq!(r.status, "failed");
        assert!(r.output_excerpt.contains("[high] src/main.rs:12"));
    }

    #[test]
    fn missing_structured_output_defaults_to_refuted() {
        let r = gate_result_from_structured(None, None, 10);
        assert_eq!(r.status, "failed");
        assert!(r.output_excerpt.contains("REFUTED"));
    }

    #[test]
    fn turn_error_maps_to_failed() {
        let r = gate_result_from_structured(None, Some("boom".into()), 10);
        assert_eq!(r.status, "failed");
        assert!(r.output_excerpt.contains("boom"));
    }

    #[test]
    fn malformed_verdict_maps_to_failed() {
        let v = serde_json::json!({"verdict": "maybe", "findings": "oops"});
        let r = gate_result_from_structured(Some(v), None, 10);
        assert_eq!(r.status, "failed");
    }
}
