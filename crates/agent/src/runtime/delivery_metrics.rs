//! 交付度量埋点:申报点验收与完成守卫结果落
//! `~/.anycode/logs/delivery-gates.jsonl`(best-effort,单行 JSON)。
//!
//! 用途:统计交付成功率(validator passed/total)与返工率(guard repair
//! 决策占比、repairs_used 分布),为交付链路优化提供可验证的度量底座。

use anycode_core::{TaskId, VerificationOutcome, VerificationReport, VerificationResult};
use std::io::Write;

pub(crate) const DELIVERY_GATES_LOG: &str = ".anycode/logs/delivery-gates.jsonl";

fn log_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(DELIVERY_GATES_LOG))
}

fn outcome_str(o: VerificationOutcome) -> &'static str {
    match o {
        VerificationOutcome::Passed => "passed",
        VerificationOutcome::TaskFailed => "task_failed",
        VerificationOutcome::EnvironmentFailed => "environment_failed",
    }
}

fn append_lines(lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Some(path) = log_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    for line in lines {
        let _ = writeln!(f, "{line}");
    }
}

/// 申报点验收:每个 validator 结果一行(纯函数,便于单测)。
pub(crate) fn declaration_lines(
    task_id: &TaskId,
    session_label: &str,
    results: &[VerificationResult],
) -> Vec<String> {
    let ts = chrono::Utc::now().to_rfc3339();
    let tid = task_id.to_string();
    results
        .iter()
        .map(|r| {
            serde_json::json!({
                "ts": ts,
                "event": "declaration_check",
                "task_id": tid,
                "session": session_label,
                "gate_id": r.gate_id,
                "validator_id": r.validator_id,
                "outcome": outcome_str(r.outcome),
                "severity": r.severity.as_str(),
                "artifact_path": r.artifact_path,
                "error_code": r.error_code,
            })
            .to_string()
        })
        .collect()
}

/// 完成守卫:每次决策一行汇总(成功率/返工率分子分母都从这里出)。
pub(crate) fn guard_verdict_line(
    task_id: &TaskId,
    session_label: &str,
    decision: &str,
    repairs_used: u32,
    report: Option<&VerificationReport>,
) -> String {
    serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": "guard_verdict",
        "task_id": task_id.to_string(),
        "session": session_label,
        "decision": decision,
        "repairs_used": repairs_used,
        "results": report.map(|r| r.results.len()).unwrap_or(0),
        "all_passed": report.is_some_and(|r| r.all_passed()),
        "p0_failed": report.map(|r| r.failed_p0().len()).unwrap_or(0),
        "failed_validators": report
            .map(|r| {
                r.results
                    .iter()
                    .filter(|x| x.outcome != VerificationOutcome::Passed)
                    .map(|x| x.validator_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    })
    .to_string()
}

pub(crate) fn record_declaration_checks(
    task_id: &TaskId,
    session_label: &str,
    results: &[VerificationResult],
) {
    append_lines(&declaration_lines(task_id, session_label, results));
}

pub(crate) fn record_guard_verdict(
    task_id: &TaskId,
    session_label: &str,
    decision: &str,
    repairs_used: u32,
    report: Option<&VerificationReport>,
) {
    append_lines(&[guard_verdict_line(
        task_id,
        session_label,
        decision,
        repairs_used,
        report,
    )]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use anycode_core::{GateSeverity, VERIFICATION_SCHEMA_VERSION};

    fn result(outcome: VerificationOutcome, severity: GateSeverity) -> VerificationResult {
        VerificationResult {
            gate_id: "html.parse.declared_html".into(),
            validator_id: "web.html_parse".into(),
            validator_version: "1".into(),
            outcome,
            severity,
            artifact_path: Some("/tmp/a.html".into()),
            artifact_hash: None,
            error_code: (outcome != VerificationOutcome::Passed).then(|| "parse_error".into()),
            diagnostics: vec![],
            evidence_paths: vec![],
        }
    }

    #[test]
    fn declaration_lines_are_single_line_json_with_metric_fields() {
        let tid = TaskId::nil();
        let lines = declaration_lines(
            &tid,
            "s1",
            &[
                result(VerificationOutcome::Passed, GateSeverity::P0),
                result(VerificationOutcome::TaskFailed, GateSeverity::P1),
            ],
        );
        assert_eq!(lines.len(), 2);
        let failed: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        assert!(!lines[1].contains('\n'));
        assert_eq!(failed["event"], "declaration_check");
        assert_eq!(failed["outcome"], "task_failed");
        assert_eq!(failed["severity"], "p1");
        assert_eq!(failed["error_code"], "parse_error");
        assert_eq!(failed["artifact_path"], "/tmp/a.html");
    }

    #[test]
    fn guard_verdict_line_summarizes_decision_and_failures() {
        let report = VerificationReport {
            schema_version: VERIFICATION_SCHEMA_VERSION,
            task_id: "t1".into(),
            gate_plan_hash: "h".into(),
            results: vec![
                result(VerificationOutcome::Passed, GateSeverity::P0),
                result(VerificationOutcome::TaskFailed, GateSeverity::P0),
            ],
        };
        let tid = TaskId::nil();
        let line = guard_verdict_line(&tid, "s1", "repair", 1, Some(&report));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["event"], "guard_verdict");
        assert_eq!(v["decision"], "repair");
        assert_eq!(v["repairs_used"], 1);
        assert_eq!(v["results"], 2);
        assert_eq!(v["all_passed"], false);
        assert_eq!(v["p0_failed"], 1);
        assert_eq!(
            v["failed_validators"],
            serde_json::json!(["web.html_parse"])
        );
    }
}
