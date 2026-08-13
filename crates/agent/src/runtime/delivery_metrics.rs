//! 交付度量埋点:申报点验收与完成守卫结果落
//! `~/.anycode/logs/delivery-gates.jsonl`(best-effort,单行 JSON)。
//!
//! 用途:统计交付成功率(validator passed/total)与返工率(guard repair
//! 决策占比、repairs_used 分布),为交付链路优化提供可验证的度量底座。

use anycode_core::{TaskId, VerificationOutcome, VerificationReport, VerificationResult};
use std::io::Write;

pub const DELIVERY_GATES_LOG: &str = ".anycode/logs/delivery-gates.jsonl";

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

/// P0.2/P2.8:守卫经产物反推兜底跑起门禁——关键词 family 误判的分子。
pub(crate) fn record_guard_fallback(
    task_id: &TaskId,
    session_label: &str,
    inferred_family: &str,
    requirements: usize,
) {
    append_lines(&[serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": "guard_fallback",
        "task_id": task_id.to_string(),
        "session": session_label,
        "inferred_family": inferred_family,
        "requirements": requirements,
    })
    .to_string()]);
}

/// P2.8:守卫未跑门禁(family 未启用且无反推信号 / plan 为空)——逃逸率分母事件。
pub(crate) fn record_guard_skipped(task_id: &TaskId, session_label: &str, reason: &str) {
    append_lines(&[serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": "guard_skipped",
        "task_id": task_id.to_string(),
        "session": session_label,
        "reason": reason,
    })
    .to_string()]);
}

/// P2.8:evidence repair 预算耗尽后仍无栈相关验证,守卫放行——「应拦未拦」。
pub(crate) fn record_verification_escape(task_id: &TaskId, session_label: &str) {
    append_lines(&[serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": "verification_escape",
        "task_id": task_id.to_string(),
        "session": session_label,
    })
    .to_string()]);
}

/// P0.1/P0.3:LLM grader(rubric+critic)每次判定一行;unavailable 单独计数。
pub(crate) fn record_grader_verdict(
    task_id: &TaskId,
    session_label: &str,
    verdict: &str,
    rubric_total: usize,
    rubric_failed: usize,
) {
    append_lines(&[serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": "grader_verdict",
        "task_id": task_id.to_string(),
        "session": session_label,
        "verdict": verdict,
        "rubric_total": rubric_total,
        "rubric_failed": rubric_failed,
    })
    .to_string()]);
}

/// P2.8 门禁度量汇总(逃逸率/返工率/兜底率),供周报与仪表盘消费。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DeliveryGatesSummary {
    pub guard_evaluations: u64,
    pub guard_complete: u64,
    pub guard_repair: u64,
    pub guard_failed: u64,
    pub guard_partial: u64,
    /// family 关键词误判后靠产物反推兜底跑起门禁的次数。
    pub fallback_used: u64,
    /// 未跑门禁的完成判定(按 reason 细分)。
    pub skipped: u64,
    pub skipped_by_reason: std::collections::BTreeMap<String, u64>,
    /// evidence repair 耗尽后无验证放行(「应拦未拦」)。
    pub verification_escapes: u64,
    pub grader_pass: u64,
    pub grader_refuted: u64,
    pub grader_unavailable: u64,
}

impl DeliveryGatesSummary {
    /// 逃逸率:(兜底 + 跳过 + 无验证放行) / (守卫评估 + 跳过)。
    pub fn escape_rate(&self) -> f64 {
        let total = self.guard_evaluations + self.skipped;
        if total == 0 {
            return 0.0;
        }
        (self.fallback_used + self.skipped + self.verification_escapes) as f64 / total as f64
    }
}

/// 解析 delivery-gates.jsonl 行(容错:坏行跳过),汇总度量。
pub fn summarize_lines(lines: impl IntoIterator<Item = String>) -> DeliveryGatesSummary {
    let mut s = DeliveryGatesSummary::default();
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match v.get("event").and_then(|e| e.as_str()) {
            Some("guard_verdict") => {
                s.guard_evaluations += 1;
                match v.get("decision").and_then(|d| d.as_str()) {
                    Some("complete") => s.guard_complete += 1,
                    Some("repair") => s.guard_repair += 1,
                    Some("failed") => s.guard_failed += 1,
                    Some("partial") => s.guard_partial += 1,
                    _ => {}
                }
            }
            Some("guard_fallback") => s.fallback_used += 1,
            Some("guard_skipped") => {
                s.skipped += 1;
                let reason = v
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                *s.skipped_by_reason.entry(reason).or_insert(0) += 1;
            }
            Some("verification_escape") => s.verification_escapes += 1,
            Some("grader_verdict") => match v.get("verdict").and_then(|x| x.as_str()) {
                Some("pass") => s.grader_pass += 1,
                Some("refuted") => s.grader_refuted += 1,
                _ => s.grader_unavailable += 1,
            },
            _ => {}
        }
    }
    s
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

    #[test]
    fn summarize_counts_events_and_escape_rate() {
        let tid = TaskId::nil();
        let lines = vec![
            guard_verdict_line(&tid, "s", "complete", 0, None),
            guard_verdict_line(&tid, "s", "repair", 1, None),
            serde_json::json!({"event":"guard_fallback","inferred_family":"web_design","requirements":3}).to_string(),
            serde_json::json!({"event":"guard_skipped","reason":"family_not_enabled"}).to_string(),
            serde_json::json!({"event":"verification_escape"}).to_string(),
            serde_json::json!({"event":"grader_verdict","verdict":"pass"}).to_string(),
            serde_json::json!({"event":"grader_verdict","verdict":"refuted"}).to_string(),
            "not json".to_string(),
        ];
        let s = summarize_lines(lines);
        assert_eq!(s.guard_evaluations, 2);
        assert_eq!(s.guard_complete, 1);
        assert_eq!(s.guard_repair, 1);
        assert_eq!(s.fallback_used, 1);
        assert_eq!(s.skipped, 1);
        assert_eq!(s.verification_escapes, 1);
        assert_eq!(s.grader_pass, 1);
        assert_eq!(s.grader_refuted, 1);
        // (1 fallback + 1 skipped + 1 escape) / (2 evals + 1 skipped) = 1.0
        assert!((s.escape_rate() - 1.0).abs() < f64::EPSILON);
    }
}
