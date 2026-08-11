//! 申报点验收(P1):交付物一经显式申报,立即按类型跑可执行验收清单;
//! 不通过就把诊断注入对话让 agent 当场返修,而不是等到完成守卫才发现。
//!
//! 与完成守卫(CompletionGuard)的关系:申报点验收是「早失败」——单文件、
//! 无 family 上下文(brand/scenario 用默认),CompletionGuard 仍在任务完成时
//! 跑完整 GatePlan,两道防线共用同一套 validator 与 GatePolicy 映射。

use super::AgentRuntime;
use anycode_core::{
    Artifact, ExpectedArtifact, GatePolicy, GateSeverity, VerificationOutcome, VerificationReport,
    VerificationResult, VERIFICATION_SCHEMA_VERSION,
};
use std::collections::HashSet;
use std::path::Path;

/// 按扩展名决定申报验收的交付物类型(与 GatePolicy 的 kind 分支对齐)。
/// 其余类型(image/video/notebook/csv 等)申报点不验,留给完成守卫或人工。
fn acceptance_kind(path: &str) -> Option<&'static str> {
    let lower = path.to_lowercase();
    let ext = Path::new(&lower).extension().and_then(|e| e.to_str())?;
    match ext {
        "html" | "htm" => Some("html"),
        "docx" => Some("docx"),
        "pptx" => Some("pptx"),
        "xlsx" => Some("xlsx"),
        "md" | "markdown" => Some("markdown"),
        "json" if lower.contains("workbook") => Some("json"),
        _ => None,
    }
}

/// 同一文件同一版本只验一次;修复后重新申报(bytes 变化)会再验。
fn dedupe_key(art: &Artifact) -> Option<String> {
    let path = art.path.as_deref()?;
    Some(format!("{path}:{}", art.bytes.unwrap_or(0)))
}

/// 把申报产物合成 ExpectedArtifact,复用 GatePolicy 的 kind→validators 映射。
fn expected_for_declared(art: &Artifact, kind: &str) -> Option<ExpectedArtifact> {
    let path = art.path.as_deref()?;
    let file_name = Path::new(path).file_name()?.to_string_lossy().to_string();
    let lower = path.to_lowercase();
    // 命中 GatePolicy 的 slide 分支:slides/ 目录下的 HTML 按幻灯片验收,
    // 不走通用网页的 h1/anchor 结构门。
    let id = if kind == "html" && (lower.contains("/slides/") || file_name.starts_with("slide-")) {
        "declared_slides_html".to_string()
    } else if kind == "json" {
        // GatePolicy json 分支要求 id 含 "workbook"。
        "declared_workbook_json".to_string()
    } else {
        format!("declared_{kind}")
    };
    Some(ExpectedArtifact {
        id,
        kind: kind.to_string(),
        required: true,
        path_globs: vec![file_name],
    })
}

pub(crate) struct DeclarationAcceptance {
    pub checked: usize,
    pub results: Vec<VerificationResult>,
}

impl DeclarationAcceptance {
    /// 有可修复失败时给 agent 的当场返修消息;环境失败不打扰(完成守卫会兜)。
    pub fn repair_message(&self) -> Option<String> {
        let actionable = self
            .results
            .iter()
            .any(|r| r.outcome == VerificationOutcome::TaskFailed);
        if !actionable {
            return None;
        }
        let report = VerificationReport {
            schema_version: VERIFICATION_SCHEMA_VERSION,
            task_id: "declaration".into(),
            gate_plan_hash: "declaration".into(),
            results: self.results.clone(),
        };
        Some(format!(
            "{}\nThe above checks ran on the deliverable you just declared. Fix the file now, \
             then re-declare it (or withdraw the declaration if it was premature).",
            report.repair_diagnostics()
        ))
    }
}

impl AgentRuntime {
    /// 对本轮新申报的交付物跑验收清单(跟随 completion_guard 开关)。
    pub(crate) async fn check_declared_deliverables(
        &self,
        new_artifacts: &[Artifact],
        checked: &mut HashSet<String>,
        workspace: &Path,
    ) -> DeclarationAcceptance {
        let mut out = DeclarationAcceptance {
            checked: 0,
            results: Vec::new(),
        };
        if !self.completion_guard.policy.enabled {
            return out;
        }
        for art in new_artifacts {
            let Some(path) = art.path.as_deref() else {
                continue;
            };
            let Some(kind) = acceptance_kind(path) else {
                continue;
            };
            let Some(key) = dedupe_key(art) else {
                continue;
            };
            if !checked.insert(key) {
                continue;
            }
            let Some(expected) = expected_for_declared(art, kind) else {
                continue;
            };
            let mut plan =
                GatePolicy::plan(None, std::slice::from_ref(&expected), "declaration", None);
            // Info 级(screenshot 取证)是观测项,申报点不跑。
            plan.requirements
                .retain(|r| r.severity != GateSeverity::Info);
            if plan.is_empty() {
                continue;
            }
            let report = self
                .completion_guard
                .registry
                .run_plan(
                    "declaration",
                    &plan,
                    std::slice::from_ref(&expected),
                    std::slice::from_ref(art),
                    &anycode_tools::ValidationContext {
                        workspace: workspace.to_path_buf(),
                        extras: plan.extras.clone(),
                    },
                )
                .await;
            out.checked += 1;
            out.results.extend(report.results);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_kind_maps_deliverable_extensions() {
        assert_eq!(acceptance_kind("/tmp/deck.pptx"), Some("pptx"));
        assert_eq!(acceptance_kind("/tmp/report.docx"), Some("docx"));
        assert_eq!(acceptance_kind("/tmp/book.xlsx"), Some("xlsx"));
        assert_eq!(acceptance_kind("/tmp/slides/page-1.html"), Some("html"));
        assert_eq!(acceptance_kind("/tmp/report.md"), Some("markdown"));
        assert_eq!(acceptance_kind("/tmp/workbook.json"), Some("json"));
        assert_eq!(acceptance_kind("/tmp/cover.png"), None);
        assert_eq!(acceptance_kind("/tmp/data.csv"), None);
        assert_eq!(acceptance_kind("/tmp/config.json"), None);
    }

    #[test]
    fn slides_dir_html_uses_slide_gate_id() {
        let art = Artifact::from_path("/tmp/ws/slides/page-1.html");
        let expected = expected_for_declared(&art, "html").unwrap();
        assert!(expected.id.contains("slide"));
        let plan = GatePolicy::plan(None, &[expected], "h", None);
        assert!(plan
            .requirements
            .iter()
            .any(|r| r.validator_id == "office.slide_html_validate"));
        assert!(!plan
            .requirements
            .iter()
            .any(|r| r.validator_id == "web.html_structure"));
    }

    #[test]
    fn generic_html_uses_web_structure_gates() {
        let art = Artifact::from_path("/tmp/ws/landing.html");
        let expected = expected_for_declared(&art, "html").unwrap();
        let plan = GatePolicy::plan(None, &[expected], "h", None);
        assert!(plan
            .requirements
            .iter()
            .any(|r| r.validator_id == "web.html_structure"));
    }

    #[test]
    fn dedupe_key_changes_with_bytes() {
        let mut art = Artifact::from_path("/tmp/a.html");
        art.bytes = Some(10);
        let k1 = dedupe_key(&art).unwrap();
        art.bytes = Some(11);
        let k2 = dedupe_key(&art).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn repair_message_only_on_task_failure() {
        let env_only = DeclarationAcceptance {
            checked: 1,
            results: vec![VerificationResult {
                gate_id: "g".into(),
                validator_id: "web.html_parse".into(),
                validator_version: "1".into(),
                outcome: VerificationOutcome::EnvironmentFailed,
                severity: GateSeverity::P0,
                artifact_path: Some("/tmp/a.html".into()),
                artifact_hash: None,
                error_code: Some("tool_missing".into()),
                diagnostics: vec![],
                evidence_paths: vec![],
            }],
        };
        assert!(env_only.repair_message().is_none());

        let task_failed = DeclarationAcceptance {
            checked: 1,
            results: vec![VerificationResult {
                outcome: VerificationOutcome::TaskFailed,
                error_code: Some("missing_h1".into()),
                diagnostics: vec!["exactly one H1 required".into()],
                ..env_only.results[0].clone()
            }],
        };
        let msg = task_failed.repair_message().unwrap();
        assert!(msg.contains("missing_h1"));
        assert!(msg.contains("re-declare"));
    }
}
