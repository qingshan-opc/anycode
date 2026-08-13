//! CompletionGuard: independent gate check before declaring a turn/task complete.
//!
//! 判定全部为确定性代码(validator 执行结果 + 预算/进展计数),模型只承担
//! 修复动作本身。三层兜底:
//! - P0.2:family 关键词误判或 plan 为空时,从产物/写文件痕迹反推 family
//!   并就地合成 GatePlan(`family_fallback`),杜绝「应跑未跑」静默逃逸。
//! - P1.6:repair 预算自适应——失败 gate 集合变化视为有进展,预算可续到
//!   `max_repairs`;集合不变(含诊断逐字相同)立即 Failed,不空转。
//! - 环境失败(工具链缺失/超时)不再进入 repair 死循环:无 TaskFailed 时
//!   归为 Partial 并显式标注环境原因。

use super::family_fallback;
use anycode_core::{
    Artifact, ExpectedArtifact, GatePlan, GatePolicy, GateSeverity, TaskFamily,
    VerificationOutcome, VerificationReport,
};
use anycode_tools::{ValidationContext, ValidatorRegistry};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardDecision {
    Complete,
    Repair,
    Partial,
    Failed,
}

#[derive(Debug, Clone)]
pub struct GuardOutcome {
    pub decision: GuardDecision,
    pub report: Option<VerificationReport>,
    pub repair_message: Option<String>,
    /// P0.2:本次评估经产物反推兜底触发的 family(用于逃逸度量)。
    pub fallback_family: Option<TaskFamily>,
    /// 守卫未跑门禁的原因(用于逃逸度量);跑过门禁(含兜底)为 None。
    pub skipped_reason: Option<&'static str>,
}

impl GuardOutcome {
    fn skipped(reason: &'static str) -> Self {
        Self {
            decision: GuardDecision::Complete,
            report: None,
            repair_message: None,
            fallback_family: None,
            skipped_reason: Some(reason),
        }
    }

    fn decided(
        decision: GuardDecision,
        report: VerificationReport,
        repair_message: Option<String>,
        fallback_family: Option<TaskFamily>,
    ) -> Self {
        Self {
            decision,
            report: Some(report),
            repair_message,
            fallback_family,
            skipped_reason: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionGuardPolicy {
    /// When false, guard is a no-op (legacy completion).
    pub enabled: bool,
    /// repair 预算上限(P1.6 自适应:有进展才续,无进展立即断)。
    pub max_repairs: u32,
    pub enabled_families: Vec<TaskFamily>,
    /// P0.2 允许产物反推兜底触发的 family。代码门禁经兜底触发可自动限定在
    /// 「确实写过代码文件」的会话,故 CrossFileCoding 只在此列出。
    pub fallback_families: Vec<TaskFamily>,
    /// P0.1/P0.3:确定性门禁全过后启用 LLM grader(rubric 语义验收 + critic
    /// 对抗复核)。LLM 传输失败不阻断完成(记逃逸度量)。
    pub grader_enabled: bool,
    /// P1.7:完成守卫期执行 `{workspace}/.anycode/gates/` 项目自定义脚本。
    pub project_gates_enabled: bool,
}

impl Default for CompletionGuardPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_repairs: 3,
            // M1: web; M3: office/docx/pptx share OfficeDelivery family.
            enabled_families: vec![TaskFamily::WebDesign, TaskFamily::OfficeDelivery],
            fallback_families: vec![
                TaskFamily::WebDesign,
                TaskFamily::OfficeDelivery,
                TaskFamily::CrossFileCoding,
            ],
            grader_enabled: true,
            project_gates_enabled: true,
        }
    }
}

pub struct CompletionGuard {
    pub registry: Arc<ValidatorRegistry>,
    pub policy: CompletionGuardPolicy,
}

impl CompletionGuard {
    pub fn new(registry: Arc<ValidatorRegistry>, policy: CompletionGuardPolicy) -> Self {
        Self { registry, policy }
    }

    pub fn family_enabled(&self, family: Option<TaskFamily>) -> bool {
        self.policy.enabled
            && family.is_some_and(|f| self.policy.enabled_families.iter().any(|x| *x == f))
    }

    fn fallback_enabled(&self, family: TaskFamily) -> bool {
        self.policy.enabled && self.policy.fallback_families.contains(&family)
    }

    pub fn grader_enabled(&self) -> bool {
        self.policy.enabled && self.policy.grader_enabled
    }

    pub async fn evaluate(
        &self,
        task_id: &str,
        family: Option<TaskFamily>,
        plan: Option<&GatePlan>,
        expected: &[ExpectedArtifact],
        artifacts: &[Artifact],
        workspace: &Path,
        repairs_used: u32,
        last_diagnostics: Option<&str>,
        last_failed_gates: &[String],
        written_paths: &[String],
    ) -> GuardOutcome {
        if !self.policy.enabled {
            return GuardOutcome::skipped("guard_disabled");
        }

        // P0.2:family 未启用或 plan 为空时,从产物/写文件痕迹反推 family 就地合成 plan。
        let fallback_plan;
        let fallback_expected;
        let mut fallback_family = None;
        let (plan, expected) = match plan.filter(|p| self.family_enabled(family) && !p.is_empty()) {
            Some(p) => (p.clone(), expected.to_vec()),
            None => {
                // 只看写工具痕迹:已申报产物在申报点已被 delivery_acceptance 验收。
                let inferred = family_fallback::infer_family_from_signals(written_paths);
                match inferred {
                    Some((f, exp)) if self.fallback_enabled(f) => {
                        fallback_family = Some(f);
                        fallback_plan =
                            GatePolicy::plan(Some(f), &exp, format!("fallback:{task_id}"), None);
                        if fallback_plan.is_empty() {
                            return GuardOutcome::skipped("fallback_plan_empty");
                        }
                        fallback_expected = exp;
                        (fallback_plan.clone(), fallback_expected.clone())
                    }
                    _ => {
                        let reason = if self.family_enabled(family) {
                            "plan_empty"
                        } else {
                            "family_not_enabled"
                        };
                        return GuardOutcome::skipped(reason);
                    }
                }
            }
        };

        let mut report = self
            .registry
            .run_plan(
                task_id,
                &plan,
                &expected,
                artifacts,
                &ValidationContext {
                    workspace: workspace.to_path_buf(),
                    extras: plan.extras.clone(),
                },
            )
            .await;

        // P1.7:项目级自定义门禁脚本(.anycode/gates/),随守卫门禁一并判定。
        if self.policy.project_gates_enabled {
            let cfg = anycode_tools::verification::project_gates::ProjectGatesConfig::from_extras(
                &plan.extras,
            );
            if let Some(results) = anycode_tools::verification::project_gates::run_project_gates(
                workspace, task_id, &cfg,
            )
            .await
            {
                report.results.extend(results);
            }
        }

        let p0_task_failed = report.results.iter().any(|r| {
            r.severity == GateSeverity::P0 && r.outcome == VerificationOutcome::TaskFailed
        });
        let p1_task_failed = report.results.iter().any(|r| {
            r.severity == GateSeverity::P1 && r.outcome == VerificationOutcome::TaskFailed
        });

        // 无任务级失败:环境失败(工具链缺失/超时)仅降级为 Partial,不进 repair 循环。
        if !p0_task_failed && !p1_task_failed {
            let decision = if report.has_blocking_environment_failure() {
                GuardDecision::Partial
            } else {
                GuardDecision::Complete
            };
            return GuardOutcome::decided(decision, report, None, fallback_family);
        }

        let diagnostics = report.repair_diagnostics();
        let failed_gates: Vec<String> = report
            .results
            .iter()
            .filter(|r| {
                r.outcome == VerificationOutcome::TaskFailed && r.severity != GateSeverity::Info
            })
            .map(|r| r.gate_id.clone())
            .collect();

        // P1.6 无进展熔断:失败 gate 集合不变(或诊断逐字重复)即停止,不空耗预算。
        let no_progress = !failed_gates.is_empty() && failed_gates == last_failed_gates;
        if no_progress || last_diagnostics.is_some_and(|prev| prev == diagnostics) {
            return GuardOutcome::decided(
                GuardDecision::Failed,
                report,
                Some(
                    "Verification made no progress between repairs (same failing gates) — stopping repair loop."
                        .into(),
                ),
                fallback_family,
            );
        }

        if repairs_used < self.policy.max_repairs {
            return GuardOutcome::decided(
                GuardDecision::Repair,
                report,
                Some(diagnostics),
                fallback_family,
            );
        }

        GuardOutcome::decided(
            if p0_task_failed {
                GuardDecision::Failed
            } else {
                GuardDecision::Partial
            },
            report,
            Some(diagnostics),
            fallback_family,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anycode_core::{GatePolicy, GateSeverity, VerificationResult};

    #[tokio::test]
    async fn requests_repair_when_html_missing() {
        let guard = CompletionGuard::new(
            Arc::new(ValidatorRegistry::new()),
            CompletionGuardPolicy::default(),
        );
        let expected = vec![ExpectedArtifact {
            id: "landing_html".into(),
            kind: "html".into(),
            required: true,
            path_globs: vec!["**/*.html".into()],
        }];
        let plan = GatePolicy::plan(Some(TaskFamily::WebDesign), &expected, "h", None);
        let temp = tempfile::tempdir().unwrap();
        let out = guard
            .evaluate(
                "t1",
                Some(TaskFamily::WebDesign),
                Some(&plan),
                &expected,
                &[],
                temp.path(),
                0,
                None,
                &[],
                &[],
            )
            .await;
        assert_eq!(out.decision, GuardDecision::Repair);
        assert!(out.repair_message.unwrap().contains("missing_artifact"));
    }

    #[tokio::test]
    async fn general_family_with_html_write_falls_back_to_web_gates() {
        // P0.2:infer_family 误判为 General,但写了 .html → 兜底跑 WebDesign 门禁。
        let guard = CompletionGuard::new(
            Arc::new(ValidatorRegistry::new()),
            CompletionGuardPolicy::default(),
        );
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            "<html><body><h1>x</h1></body></html>",
        )
        .unwrap();
        let out = guard
            .evaluate(
                "t1",
                Some(TaskFamily::General),
                None,
                &[],
                &[],
                temp.path(),
                0,
                None,
                &[],
                &[temp
                    .path()
                    .join("index.html")
                    .to_string_lossy()
                    .into_owned()],
            )
            .await;
        assert_eq!(out.fallback_family, Some(TaskFamily::WebDesign));
        assert!(out.report.is_some());
        assert!(out.skipped_reason.is_none());
    }

    #[tokio::test]
    async fn pure_text_session_skips_with_reason() {
        let guard = CompletionGuard::new(
            Arc::new(ValidatorRegistry::new()),
            CompletionGuardPolicy::default(),
        );
        let temp = tempfile::tempdir().unwrap();
        let out = guard
            .evaluate(
                "t1",
                Some(TaskFamily::General),
                None,
                &[],
                &[],
                temp.path(),
                0,
                None,
                &[],
                &[],
            )
            .await;
        assert_eq!(out.decision, GuardDecision::Complete);
        assert_eq!(out.skipped_reason, Some("family_not_enabled"));
        assert!(out.fallback_family.is_none());
    }

    #[tokio::test]
    async fn rust_write_falls_back_to_code_gates() {
        // P0.4:写了 .rs → CrossFileCoding 兜底;workspace 无 Cargo.toml → 探测不到栈 → Passed。
        let guard = CompletionGuard::new(
            Arc::new(ValidatorRegistry::new()),
            CompletionGuardPolicy::default(),
        );
        let temp = tempfile::tempdir().unwrap();
        let out = guard
            .evaluate(
                "t1",
                Some(TaskFamily::General),
                None,
                &[],
                &[],
                temp.path(),
                0,
                None,
                &[],
                &[temp.path().join("main.rs").to_string_lossy().into_owned()],
            )
            .await;
        assert_eq!(out.fallback_family, Some(TaskFamily::CrossFileCoding));
        let report = out.report.unwrap();
        assert!(report
            .results
            .iter()
            .any(|r| r.validator_id == "code.stack_verify"));
    }

    #[tokio::test]
    async fn no_progress_same_gates_fails_immediately() {
        // P1.6:失败 gate 集合不变 → 不消耗剩余预算,直接 Failed。
        let guard = CompletionGuard::new(
            Arc::new(ValidatorRegistry::new()),
            CompletionGuardPolicy::default(),
        );
        let expected = vec![ExpectedArtifact {
            id: "landing_html".into(),
            kind: "html".into(),
            required: true,
            path_globs: vec!["**/*.html".into()],
        }];
        let plan = GatePolicy::plan(Some(TaskFamily::WebDesign), &expected, "h", None);
        let temp = tempfile::tempdir().unwrap();
        // 第一轮:正常进入 Repair,并拿到真实的失败 gate 集合与诊断。
        let first = guard
            .evaluate(
                "t1",
                Some(TaskFamily::WebDesign),
                Some(&plan),
                &expected,
                &[],
                temp.path(),
                0,
                None,
                &[],
                &[],
            )
            .await;
        assert_eq!(first.decision, GuardDecision::Repair);
        let report = first.report.as_ref().unwrap();
        let gates: Vec<String> = report
            .results
            .iter()
            .filter(|r| {
                r.outcome == VerificationOutcome::TaskFailed && r.severity != GateSeverity::Info
            })
            .map(|r| r.gate_id.clone())
            .collect();
        let diag = report.repair_diagnostics();
        // 第二轮:gate 集合与诊断均无变化 → 无进展熔断,即使预算未满也 Failed。
        let out = guard
            .evaluate(
                "t1",
                Some(TaskFamily::WebDesign),
                Some(&plan),
                &expected,
                &[],
                temp.path(),
                1,
                Some(&diag),
                &gates,
                &[],
            )
            .await;
        assert_eq!(out.decision, GuardDecision::Failed);
    }

    #[test]
    fn report_helpers_still_hold() {
        let r = VerificationResult {
            gate_id: "g".into(),
            validator_id: "v".into(),
            validator_version: "1".into(),
            outcome: VerificationOutcome::EnvironmentFailed,
            severity: GateSeverity::P0,
            artifact_path: None,
            artifact_hash: None,
            error_code: Some("gate_timeout".into()),
            diagnostics: vec![],
            evidence_paths: vec![],
        };
        let report = VerificationReport {
            schema_version: 2,
            task_id: "t".into(),
            gate_plan_hash: "h".into(),
            results: vec![r],
        };
        assert!(report.has_blocking_environment_failure());
    }
}
