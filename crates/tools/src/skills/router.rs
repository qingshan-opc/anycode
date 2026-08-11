//! Capability → skill resolution (pure function; no side effects).
//!
//! Skills are **recommended, never preloaded**: the full catalog listing is
//! already injected into the system prompt at bootstrap
//! (`crates/bootstrap/src/prompt_runtime.rs`), so this module only emits a
//! compact recommendation segment. The model loads full instructions on
//! demand via the `Skill` tool when it judges the recommendation relevant —
//! keyword-inferred routing must not force a production SOP onto creative
//! user intent.

use super::effective::SkillsGovernance;
use super::{SkillCatalog, SkillMeta};

#[derive(Debug, Clone)]
pub struct SkillResolutionContext {
    pub agent_type: String,
    pub project_root: Option<std::path::PathBuf>,
    pub platform: String,
    pub production_skills_enabled: bool,
}

impl Default for SkillResolutionContext {
    fn default() -> Self {
        Self {
            agent_type: "general-purpose".into(),
            project_root: None,
            platform: std::env::consts::OS.to_string(),
            production_skills_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillMatchStatus {
    Selected,
    Unresolved,
    Denied,
    Disabled,
    DependencyUnavailable,
}

#[derive(Debug, Clone)]
pub struct SelectedSkill {
    pub capability: String,
    pub skill_id: String,
    pub status: SkillMatchStatus,
    /// One-line skill description from the catalog (empty when unresolved).
    pub description: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SkillResolution {
    pub selected: Vec<SelectedSkill>,
    pub prompt_segment: String,
    pub denied_skill_ids: Vec<String>,
}

fn platform_ok(meta: &SkillMeta, platform: &str) -> bool {
    meta.platforms.is_empty()
        || meta
            .platforms
            .iter()
            .any(|p| p.eq_ignore_ascii_case(platform))
}

/// Resolve required capabilities to governed skills.
///
/// Sort key (unique): exact capability → governance allow → platform/deps →
/// priority desc → skill id asc.
pub fn resolve_capabilities(
    required: &[String],
    catalog: &SkillCatalog,
    governance: &SkillsGovernance,
    context: &SkillResolutionContext,
) -> SkillResolution {
    let mut out = SkillResolution::default();
    if !context.production_skills_enabled {
        for cap in required {
            out.selected.push(SelectedSkill {
                capability: cap.clone(),
                skill_id: String::new(),
                status: SkillMatchStatus::Disabled,
                description: String::new(),
                candidates: vec![],
            });
        }
        // Deny all production skill ids so models cannot guess around the arm.
        out.denied_skill_ids = catalog.metas().iter().map(|m| m.id.clone()).collect();
        return out;
    }

    for cap in required {
        let mut matches: Vec<&SkillMeta> = catalog
            .metas()
            .iter()
            .filter(|m| m.provides_capabilities.iter().any(|c| c == cap))
            .collect();
        matches.sort_by(|a, b| {
            let ga = governance.is_allowed(&context.agent_type, &a.id);
            let gb = governance.is_allowed(&context.agent_type, &b.id);
            gb.cmp(&ga)
                .then_with(|| {
                    platform_ok(b, &context.platform).cmp(&platform_ok(a, &context.platform))
                })
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a.id.cmp(&b.id))
        });
        let candidates: Vec<String> = matches.iter().map(|m| m.id.clone()).collect();
        let Some(best) = matches.first().copied() else {
            out.selected.push(SelectedSkill {
                capability: cap.clone(),
                skill_id: String::new(),
                status: SkillMatchStatus::Unresolved,
                description: String::new(),
                candidates,
            });
            continue;
        };
        if !governance.is_allowed(&context.agent_type, &best.id) {
            out.selected.push(SelectedSkill {
                capability: cap.clone(),
                skill_id: best.id.clone(),
                status: SkillMatchStatus::Denied,
                description: String::new(),
                candidates,
            });
            out.denied_skill_ids.push(best.id.clone());
            continue;
        }
        if !platform_ok(best, &context.platform) {
            out.selected.push(SelectedSkill {
                capability: cap.clone(),
                skill_id: best.id.clone(),
                status: SkillMatchStatus::DependencyUnavailable,
                description: String::new(),
                candidates,
            });
            continue;
        }
        out.selected.push(SelectedSkill {
            capability: cap.clone(),
            skill_id: best.id.clone(),
            status: SkillMatchStatus::Selected,
            description: best.description.clone(),
            candidates,
        });
    }

    let mut lines = Vec::new();
    for sel in &out.selected {
        if sel.status != SkillMatchStatus::Selected || sel.skill_id.is_empty() {
            continue;
        }
        if lines.is_empty() {
            lines.push("## Recommended Skills".to_string());
            lines.push(
                "Keyword-inferred matches — recommendations, not mandates. Load full \
                 instructions with the **Skill** tool (`{\"name\": \"<id>\"}`) when relevant, \
                 use **SkillSearch** for alternatives, or proceed without a skill if your \
                 reading of the task differs. The user's explicit instructions always take \
                 precedence over any skill SOP."
                    .into(),
            );
        }
        lines.push(format!(
            "- `{}` (for capability `{}`): {}",
            sel.skill_id, sel.capability, sel.description
        ));
    }
    out.prompt_segment = lines.join("\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::SkillCatalog;
    use std::fs;
    use std::path::PathBuf;

    fn write_skill(root: &std::path::Path, id: &str, caps: &str, priority: i32) {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!(
                "---\nname: {id}\ndescription: test\nprovides_capabilities: [{caps}]\npriority: {priority}\n---\n\n# {id}\n\nDo work.\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn picks_higher_priority_then_id() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "web-a", "web.implement", 10);
        write_skill(temp.path(), "web-b", "web.implement", 50);
        let catalog = SkillCatalog::scan(&[temp.path().to_path_buf()], None, 120_000, false);
        let gov = SkillsGovernance::default();
        let ctx = SkillResolutionContext::default();
        let res = resolve_capabilities(&["web.implement".into()], &catalog, &gov, &ctx);
        assert_eq!(res.selected[0].skill_id, "web-b");
        assert_eq!(res.selected[0].status, SkillMatchStatus::Selected);
        assert!(!res.prompt_segment.is_empty());
    }

    #[test]
    fn recommendations_do_not_preload_sop_instructions() {
        // 预载 SOP 会把关键词路由的猜测强加给用户意图——segment 只应包含
        // 推荐清单,绝不能内联 skill 正文。
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "web-a", "web.implement", 10);
        let catalog = SkillCatalog::scan(&[temp.path().to_path_buf()], None, 120_000, false);
        let gov = SkillsGovernance::default();
        let ctx = SkillResolutionContext::default();
        let res = resolve_capabilities(&["web.implement".into()], &catalog, &gov, &ctx);
        assert!(res.prompt_segment.starts_with("## Recommended Skills"));
        assert!(
            !res.prompt_segment.contains("Do work."),
            "prompt segment must not inline skill instructions: {}",
            res.prompt_segment
        );
        assert!(
            res.prompt_segment.contains("recommendations, not mandates"),
            "segment must frame skills as overridable recommendations: {}",
            res.prompt_segment
        );
        assert!(res.prompt_segment.contains("`web-a`"));
    }

    #[test]
    fn commercial_delivery_wins_pptx_export() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "legacy-pptx", "presentation.export.pptx", 50);
        write_skill(
            temp.path(),
            "presentation-commercial-delivery",
            "presentation.export.pptx",
            130,
        );
        let catalog = SkillCatalog::scan(&[temp.path().to_path_buf()], None, 120_000, false);
        let gov = SkillsGovernance::default();
        let ctx = SkillResolutionContext::default();
        let res = resolve_capabilities(&["presentation.export.pptx".into()], &catalog, &gov, &ctx);
        assert_eq!(res.selected[0].skill_id, "presentation-commercial-delivery");
    }

    #[test]
    fn anycode_ppt_author_prefers_anycode_skill() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "legacy-pptx", "presentation.author", 50);
        write_skill(temp.path(), "anycode-ppt", "presentation.author", 125);
        let catalog = SkillCatalog::scan(&[temp.path().to_path_buf()], None, 120_000, false);
        let gov = SkillsGovernance::default();
        let ctx = SkillResolutionContext::default();
        let res = resolve_capabilities(&["presentation.author".into()], &catalog, &gov, &ctx);
        assert_eq!(res.selected[0].skill_id, "anycode-ppt");
    }

    #[test]
    fn anycode_docx_author_prefers_anycode_skill() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "legacy-docx", "document.author", 90);
        write_skill(temp.path(), "anycode-docx", "document.author", 125);
        let catalog = SkillCatalog::scan(&[temp.path().to_path_buf()], None, 120_000, false);
        let gov = SkillsGovernance::default();
        let ctx = SkillResolutionContext::default();
        let res = resolve_capabilities(&["document.author".into()], &catalog, &gov, &ctx);
        assert_eq!(res.selected[0].skill_id, "anycode-docx");
    }

    #[test]
    fn anycode_xlsx_author_prefers_anycode_skill() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "legacy-xlsx", "spreadsheet.author", 90);
        write_skill(temp.path(), "anycode-xlsx", "spreadsheet.author", 125);
        let catalog = SkillCatalog::scan(&[temp.path().to_path_buf()], None, 120_000, false);
        let gov = SkillsGovernance::default();
        let ctx = SkillResolutionContext::default();
        let res = resolve_capabilities(&["spreadsheet.author".into()], &catalog, &gov, &ctx);
        assert_eq!(res.selected[0].skill_id, "anycode-xlsx");
    }

    #[test]
    fn disabled_arm_denies_all() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "web-a", "web.implement", 10);
        let catalog = SkillCatalog::scan(&[PathBuf::from(temp.path())], None, 120_000, false);
        let gov = SkillsGovernance::default();
        let mut ctx = SkillResolutionContext::default();
        ctx.production_skills_enabled = false;
        let res = resolve_capabilities(&["web.implement".into()], &catalog, &gov, &ctx);
        assert_eq!(res.selected[0].status, SkillMatchStatus::Disabled);
        assert!(res.denied_skill_ids.contains(&"web-a".into()));
    }
}
