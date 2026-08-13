//! `PlanWrite` — hierarchical session plan tree (in-memory + optional DB persist).

use crate::services::ToolServices;
use crate::session_store::resolve_session_key;
use anycode_core::prelude::*;
use anycode_core::{
    apply_plan_patches, format_plan_tree_summary, parse_plan_doc, plan_tree_all_completed,
    plan_tree_current_focus, plan_tree_in_progress_leaf_count, plan_tree_is_empty,
    plan_tree_next_pending, rollup_plan_statuses, validate_plan_tree, PlanLimits, PlanNode,
    PlanNodeKind, PlanPatch, PlanStatus, PlanTree, PlanValidationError,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Instant;

pub struct PlanWriteTool {
    services: Arc<ToolServices>,
}

impl PlanWriteTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Deserialize)]
struct PlanNodeIn {
    id: String,
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    children: Vec<PlanNodeIn>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PwInput {
    /// Canonical input: a multi-level Markdown plan document.
    #[serde(default)]
    doc: Option<String>,
    /// Update only the guidance prose at the top of the document.
    #[serde(default)]
    prose: Option<String>,
    /// Legacy JSON tree input (kept for backward compatibility).
    #[serde(default)]
    tree: Option<Vec<PlanNodeIn>>,
    #[serde(default)]
    updates: Option<Vec<PlanPatch>>,
}

fn map_node_in(node: PlanNodeIn) -> Result<PlanNode, CoreError> {
    let status = node
        .status
        .as_deref()
        .and_then(PlanStatus::parse)
        .unwrap_or_default();
    let kind = node.kind.as_deref().and_then(PlanNodeKind::parse);
    let children = node
        .children
        .into_iter()
        .map(map_node_in)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PlanNode {
        id: node.id,
        title: node.title,
        status,
        children,
        detail: node.detail,
        kind,
    })
}

fn map_tree_in(nodes: Vec<PlanNodeIn>) -> Result<PlanTree, CoreError> {
    let roots = nodes
        .into_iter()
        .map(map_node_in)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PlanTree {
        prose: String::new(),
        roots,
    })
}

fn validation_err(e: PlanValidationError) -> String {
    e.message
}

#[async_trait]
impl Tool for PlanWriteTool {
    fn name(&self) -> &str {
        "PlanWrite"
    }

    fn description(&self) -> &str {
        "Update the session plan — a multi-level Markdown document, not JSON. Use `doc` for the initial plan or full revisions: the top of the document is free-form Markdown guidance (goal, strategy, constraints, acceptance criteria — written like a short instruction manual), and the bottom is the detailed multi-level tree as a nested checkbox list: `- [ ] Title `(id)` — optional detail`, children indented by 2 spaces. Status glyphs: [ ] pending, [~] in_progress, [x] completed, [X] failed, [!] blocked, [-] cancelled. Use `updates` for status changes by id (cheaper than rewriting `doc`), and `prose` to revise only the guidance section. Status rules: mark a leaf in_progress before starting it and completed immediately when done; keep at most ONE in_progress leaf at a time; update leaves only — parent status is derived automatically. Skip planning for simple tasks. The result echoes a compact summary with Current (active node) and Next (upcoming leaf)."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "doc": {
                    "type": "string",
                    "description": "Replace the entire plan with this Markdown document. Top: guidance prose (Markdown, instruction-manual style). Bottom: the multi-level tree as a nested list, e.g.\n# 计划：重构鉴权模块\n\n先调研、后实施，改完跑全量测试。\n\n- [ ] 调研现状 `(research)` — 只读\n  - [~] 阅读 auth 模块 `(read-auth)`\n  - [ ] 整理调用方 `(list-callers)`\n- [ ] 实施 `(impl)`\nChildren indent by 2 spaces; ids are stable slugs in backtick-parens; detail follows an em dash."
                },
                "prose": {
                    "type": "string",
                    "description": "Replace only the guidance prose at the top of the plan document; the tree is untouched."
                },
                "updates": {
                    "type": "array",
                    "description": "Incremental patches: update status/title/detail by id, or add child via parent_id+node.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "parent_id": { "type": "string" },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "blocked", "failed", "cancelled"]
                            },
                            "title": { "type": "string" },
                            "detail": { "type": "string" },
                            "node": { "$ref": "#/$defs/planNode" }
                        }
                    }
                }
            },
            "$defs": {
                "planNode": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "title": { "type": "string" },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "in_progress", "completed", "blocked", "failed", "cancelled"]
                        },
                        "children": {
                            "type": "array",
                            "items": { "$ref": "#/$defs/planNode" }
                        },
                        "detail": { "type": "string" },
                        "kind": {
                            "type": "string",
                            "enum": ["phase", "task", "verify", "checkpoint"]
                        }
                    },
                    "required": ["id", "title"]
                }
            }
        })
    }

    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Auto
    }

    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }

    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let session_id = input.dashboard_session_id.clone();
        let session_key = resolve_session_key(session_id.as_deref());
        self.services.hydrate_plan_tree(session_id.as_deref()).await;
        let pw: PwInput =
            serde_json::from_value(input.input).map_err(CoreError::SerializationError)?;
        if pw.doc.is_none() && pw.tree.is_none() && pw.prose.is_none() && pw.updates.is_none() {
            return Ok(ToolOutput {
                result: serde_json::json!({ "error": "PlanWrite requires `doc`, `prose` or `updates`" }),
                error: Some("missing_input".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
        let limits = PlanLimits::default_mvp();
        let result = if let Some(doc) = pw.doc {
            let mut tree = match parse_plan_doc(&doc) {
                Ok(tree) => tree,
                Err(e) => {
                    return Ok(ToolOutput {
                        result: serde_json::json!({ "error": validation_err(e) }),
                        error: Some("validation".into()),
                        duration_ms: start.elapsed().as_millis() as u64,
                    });
                }
            };
            if let Err(e) = validate_plan_tree(&tree, &limits) {
                return Ok(ToolOutput {
                    result: serde_json::json!({ "error": validation_err(e) }),
                    error: Some("validation".into()),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
            rollup_plan_statuses(&mut tree);
            self.services.replace_plan_tree(session_id.as_deref(), tree)
        } else if let Some(tree_in) = pw.tree {
            // Legacy JSON tree input.
            let mut tree = map_tree_in(tree_in)?;
            if let Err(e) = validate_plan_tree(&tree, &limits) {
                return Ok(ToolOutput {
                    result: serde_json::json!({ "error": validation_err(e) }),
                    error: Some("validation".into()),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
            rollup_plan_statuses(&mut tree);
            self.services.replace_plan_tree(session_id.as_deref(), tree)
        } else {
            let updates = pw.updates.unwrap_or_default();
            let mut tree = self.services.plan_tree(session_id.as_deref());
            if let Some(prose) = pw.prose {
                tree.prose = prose.trim().to_string();
            }
            if let Err(e) = apply_plan_patches(&mut tree, &updates) {
                return Ok(ToolOutput {
                    result: serde_json::json!({ "error": validation_err(e) }),
                    error: Some("validation".into()),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
            if let Err(e) = validate_plan_tree(&tree, &limits) {
                return Ok(ToolOutput {
                    result: serde_json::json!({ "error": validation_err(e) }),
                    error: Some("validation".into()),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
            rollup_plan_statuses(&mut tree);
            self.services.replace_plan_tree(session_id.as_deref(), tree)
        };
        let (_old, new) = result;
        self.services
            .persist_plan_tree(session_id.as_deref(), &new)
            .await;
        let summary = format_plan_tree_summary(&new);
        // 引导而非强制：多个 in_progress 叶子时返回警告，由模型自我纠正。
        let mut warnings: Vec<String> = Vec::new();
        let in_progress = plan_tree_in_progress_leaf_count(&new);
        if in_progress > 1 {
            warnings.push(format!(
                "{in_progress} leaves are in_progress; keep at most one — finish or requeue the others to pending."
            ));
        }
        Ok(ToolOutput {
            result: serde_json::json!({
                "summary": summary,
                "currentFocus": plan_tree_current_focus(&new),
                "nextPending": plan_tree_next_pending(&new),
                "allCompleted": plan_tree_all_completed(&new),
                "cleared": plan_tree_is_empty(&new),
                "sessionId": session_key,
                "warnings": warnings,
            }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ToolServices;
    use serde_json::json;

    #[tokio::test]
    async fn plan_write_replaces_tree_for_session() {
        let services = Arc::new(ToolServices::default());
        let tool = PlanWriteTool::new(services.clone());
        let out = tool
            .execute(ToolInput {
                name: "PlanWrite".into(),
                input: json!({
                    "tree": [{
                        "id": "root",
                        "title": "Implement feature",
                        "status": "pending",
                        "children": [{
                            "id": "step-1",
                            "title": "Design",
                            "status": "pending"
                        }]
                    }]
                }),
                working_directory: Some(".".into()),
                sandbox_mode: false,
                dashboard_session_id: Some("sess_test".into()),
                task_id: None,
            })
            .await
            .unwrap();
        assert!(out.result["summary"]
            .as_str()
            .unwrap()
            .contains("Implement feature"));
        assert_eq!(
            services.plan_tree(Some("sess_test")).roots[0].title,
            "Implement feature"
        );
        assert!(plan_tree_is_empty(&services.plan_tree(Some("other"))));
    }

    fn tool_input(input: serde_json::Value) -> ToolInput {
        ToolInput {
            name: "PlanWrite".into(),
            input,
            working_directory: Some(".".into()),
            sandbox_mode: false,
            dashboard_session_id: Some("sess_guide".into()),
            task_id: None,
        }
    }

    #[tokio::test]
    async fn plan_write_result_guides_focus_and_next() {
        let services = Arc::new(ToolServices::default());
        let tool = PlanWriteTool::new(services);
        let out = tool
            .execute(tool_input(json!({
                "tree": [{
                    "id": "phase-1",
                    "title": "Build",
                    "children": [
                        { "id": "t1", "title": "Read code", "status": "in_progress" },
                        { "id": "t2", "title": "Write tests" }
                    ]
                }]
            })))
            .await
            .unwrap();
        assert_eq!(out.result["currentFocus"], "Build / Read code");
        assert_eq!(out.result["nextPending"], "Build / Write tests");
        assert!(out.result["warnings"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn plan_write_warns_on_multiple_in_progress() {
        let services = Arc::new(ToolServices::default());
        let tool = PlanWriteTool::new(services);
        let out = tool
            .execute(tool_input(json!({
                "tree": [{
                    "id": "phase-1",
                    "title": "Build",
                    "children": [
                        { "id": "t1", "title": "A", "status": "in_progress" },
                        { "id": "t2", "title": "B", "status": "in_progress" }
                    ]
                }]
            })))
            .await
            .unwrap();
        let warnings = out.result["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].as_str().unwrap().contains("at most one"));
    }

    #[tokio::test]
    async fn plan_write_accepts_markdown_doc() {
        let services = Arc::new(ToolServices::default());
        let tool = PlanWriteTool::new(services.clone());
        let doc = "# 计划：落地新计费\n\n先改库表，再改扣费；每步跑测试。\n\n- [ ] 库表 `(db)` — 新迁移\n  - [~] 写迁移 `(migration)`\n  - [ ] 回填数据 `(backfill)`\n- [ ] 扣费逻辑 `(charge)`\n";
        let out = tool
            .execute(tool_input(json!({ "doc": doc })))
            .await
            .unwrap();
        assert_eq!(out.result["currentFocus"], "库表 / 写迁移");
        assert_eq!(out.result["nextPending"], "库表 / 回填数据");
        let tree = services.plan_tree(Some("sess_guide"));
        assert!(tree.prose.contains("先改库表"));
        assert_eq!(tree.roots.len(), 2);
        assert_eq!(tree.roots[0].children[0].id, "migration");

        // prose-only update keeps the tree intact
        let out = tool
            .execute(tool_input(json!({ "prose": "修订：先小流量灰度。" })))
            .await
            .unwrap();
        assert_eq!(out.result["currentFocus"], "库表 / 写迁移");
        let tree = services.plan_tree(Some("sess_guide"));
        assert_eq!(tree.prose, "修订：先小流量灰度。");
        assert_eq!(tree.roots.len(), 2);
    }
}
