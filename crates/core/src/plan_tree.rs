//! Hierarchical plan tree for structured agent planning.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Default maximum nesting depth for plan trees.
pub const PLAN_TREE_MAX_DEPTH: usize = 4;
/// Default maximum node count for plan trees.
pub const PLAN_TREE_MAX_NODES: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Blocked,
    Failed,
    Cancelled,
}

impl PlanStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "in_progress" | "inprogress" | "running" => Some(Self::InProgress),
            "completed" | "done" => Some(Self::Completed),
            "blocked" => Some(Self::Blocked),
            "failed" => Some(Self::Failed),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Pending => "[ ]",
            Self::InProgress => "[~]",
            Self::Completed => "[x]",
            Self::Blocked => "[!]",
            Self::Failed => "[X]",
            Self::Cancelled => "[-]",
        }
    }

    /// Inverse of `glyph` (Markdown plan documents). Note `[x]` (lowercase) is
    /// completed while `[X]` (uppercase) is failed.
    pub fn from_glyph(glyph: &str) -> Option<Self> {
        match glyph.trim() {
            "[ ]" => Some(Self::Pending),
            "[~]" => Some(Self::InProgress),
            "[x]" => Some(Self::Completed),
            "[X]" => Some(Self::Failed),
            "[!]" => Some(Self::Blocked),
            "[-]" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanNodeKind {
    Phase,
    Task,
    Verify,
    Checkpoint,
}

impl PlanNodeKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "phase" => Some(Self::Phase),
            "task" => Some(Self::Task),
            "verify" => Some(Self::Verify),
            "checkpoint" => Some(Self::Checkpoint),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlanNode {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: PlanStatus,
    #[serde(default)]
    pub children: Vec<PlanNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<PlanNodeKind>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanTree {
    /// Free-form Markdown guidance that sits at the top of the plan document
    /// (goal, strategy, constraints — the "instruction manual" for the tree).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prose: String,
    #[serde(default)]
    pub roots: Vec<PlanNode>,
}

#[derive(Debug, Clone)]
pub struct PlanValidationError {
    pub message: String,
}

impl PlanValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanLimits {
    pub max_depth: usize,
    pub max_nodes: usize,
}

impl PlanLimits {
    pub fn default_mvp() -> Self {
        Self {
            max_depth: PLAN_TREE_MAX_DEPTH,
            max_nodes: PLAN_TREE_MAX_NODES,
        }
    }
}

/// Validate tree shape: unique ids, non-empty titles, depth and node limits.
pub fn validate_plan_tree(tree: &PlanTree, limits: &PlanLimits) -> Result<(), PlanValidationError> {
    let mut ids = HashSet::new();
    let mut count = 0usize;
    for root in &tree.roots {
        validate_node(root, 1, limits, &mut ids, &mut count)?;
    }
    Ok(())
}

fn validate_node(
    node: &PlanNode,
    depth: usize,
    limits: &PlanLimits,
    ids: &mut HashSet<String>,
    count: &mut usize,
) -> Result<(), PlanValidationError> {
    if depth > limits.max_depth {
        return Err(PlanValidationError::new(format!(
            "plan tree exceeds max depth {} at node '{}'",
            limits.max_depth, node.id
        )));
    }
    *count += 1;
    if *count > limits.max_nodes {
        return Err(PlanValidationError::new(format!(
            "plan tree exceeds max node count {}",
            limits.max_nodes
        )));
    }
    let id = node.id.trim();
    if id.is_empty() {
        return Err(PlanValidationError::new("plan node id must not be empty"));
    }
    if !ids.insert(id.to_string()) {
        return Err(PlanValidationError::new(format!(
            "duplicate plan node id '{}'",
            id
        )));
    }
    if node.title.trim().is_empty() {
        return Err(PlanValidationError::new(format!(
            "plan node '{}' title must not be empty",
            id
        )));
    }
    for child in &node.children {
        validate_node(child, depth + 1, limits, ids, count)?;
    }
    Ok(())
}

/// Roll up parent status from children when parent is not terminal.
pub fn rollup_plan_statuses(tree: &mut PlanTree) {
    for root in &mut tree.roots {
        rollup_node(root);
    }
}

fn rollup_node(node: &mut PlanNode) {
    for child in &mut node.children {
        rollup_node(child);
    }
    if node.children.is_empty() {
        return;
    }
    if matches!(
        node.status,
        PlanStatus::Completed | PlanStatus::Failed | PlanStatus::Cancelled
    ) {
        return;
    }
    let child_statuses: Vec<PlanStatus> = node.children.iter().map(|c| c.status).collect();
    node.status = rollup_from_children(&child_statuses);
}

fn rollup_from_children(children: &[PlanStatus]) -> PlanStatus {
    if children.iter().any(|s| matches!(s, PlanStatus::Failed)) {
        return PlanStatus::Failed;
    }
    if children.iter().any(|s| matches!(s, PlanStatus::Blocked)) {
        return PlanStatus::Blocked;
    }
    if children.iter().any(|s| matches!(s, PlanStatus::InProgress)) {
        return PlanStatus::InProgress;
    }
    if children.iter().all(|s| matches!(s, PlanStatus::Completed)) {
        return PlanStatus::Completed;
    }
    if children
        .iter()
        .all(|s| matches!(s, PlanStatus::Completed | PlanStatus::Cancelled))
    {
        return PlanStatus::Completed;
    }
    PlanStatus::Pending
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanPatch {
    pub id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub node: Option<PlanNode>,
}

/// Apply incremental patches to an existing tree.
pub fn apply_plan_patches(
    tree: &mut PlanTree,
    patches: &[PlanPatch],
) -> Result<(), PlanValidationError> {
    for patch in patches {
        if let Some(node) = &patch.node {
            if let Some(parent_id) = patch.parent_id.as_deref() {
                if !insert_child(tree, parent_id, node.clone())? {
                    return Err(PlanValidationError::new(format!(
                        "parent node '{}' not found for patch",
                        parent_id
                    )));
                }
            } else if patch.id.is_none() {
                tree.roots.push(node.clone());
            } else {
                return Err(PlanValidationError::new(
                    "patch with `node` requires `parent_id` or omit `id` to append root",
                ));
            }
            continue;
        }
        let Some(id) = patch.id.as_deref() else {
            return Err(PlanValidationError::new(
                "patch requires `id` when updating status/title/detail",
            ));
        };
        let Some(node) = find_node_mut(tree, id) else {
            return Err(PlanValidationError::new(format!(
                "plan node '{}' not found",
                id
            )));
        };
        if let Some(status) = &patch.status {
            node.status = PlanStatus::parse(status).ok_or_else(|| {
                PlanValidationError::new(format!("invalid plan status '{}'", status))
            })?;
        }
        if let Some(title) = &patch.title {
            if title.trim().is_empty() {
                return Err(PlanValidationError::new(format!(
                    "plan node '{}' title must not be empty",
                    id
                )));
            }
            node.title = title.trim().to_string();
        }
        if let Some(detail) = patch.detail.as_ref() {
            let trimmed = detail.trim();
            node.detail = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
    }
    Ok(())
}

fn find_node_mut<'a>(tree: &'a mut PlanTree, id: &str) -> Option<&'a mut PlanNode> {
    for root in &mut tree.roots {
        if let Some(found) = find_in_node_mut(root, id) {
            return Some(found);
        }
    }
    None
}

fn find_in_node_mut<'a>(node: &'a mut PlanNode, id: &str) -> Option<&'a mut PlanNode> {
    if node.id == id {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_in_node_mut(child, id) {
            return Some(found);
        }
    }
    None
}

fn insert_child(
    tree: &mut PlanTree,
    parent_id: &str,
    child: PlanNode,
) -> Result<bool, PlanValidationError> {
    if let Some(parent) = find_node_mut(tree, parent_id) {
        parent.children.push(child);
        return Ok(true);
    }
    Ok(false)
}

/// Prefix for hidden runtime context messages that carry the active plan tree.
pub const PLAN_TREE_CONTEXT_PREFIX: &str = "## Active Plan Tree";

/// Depth-first path of the deepest in-progress node, e.g. "Phase 1 / Build UI".
/// Parents derive `in_progress` via rollup, so the deepest match is the real focus.
pub fn plan_tree_current_focus(tree: &PlanTree) -> Option<String> {
    for root in &tree.roots {
        if let Some(path) = deepest_in_progress_path(root, vec![root.title.as_str()]) {
            return Some(path.join(" / "));
        }
    }
    None
}

fn deepest_in_progress_path<'a>(node: &'a PlanNode, path: Vec<&'a str>) -> Option<Vec<&'a str>> {
    // Descend regardless of parent status: an un-rolled-up parent may still
    // contain an in-progress leaf, and the deepest match is the real focus.
    for child in &node.children {
        let mut child_path = path.clone();
        child_path.push(child.title.as_str());
        if let Some(found) = deepest_in_progress_path(child, child_path) {
            return Some(found);
        }
    }
    if matches!(node.status, PlanStatus::InProgress) {
        return Some(path);
    }
    None
}

/// Depth-first path of the first actionable pending leaf.
pub fn plan_tree_next_pending(tree: &PlanTree) -> Option<String> {
    for root in &tree.roots {
        if let Some(path) = first_pending_leaf_path(root, vec![root.title.as_str()]) {
            return Some(path.join(" / "));
        }
    }
    None
}

fn first_pending_leaf_path<'a>(node: &'a PlanNode, path: Vec<&'a str>) -> Option<Vec<&'a str>> {
    // Only leaf status matters: a parent may be in-progress while later
    // siblings are still pending.
    if node.children.is_empty() {
        return matches!(node.status, PlanStatus::Pending).then_some(path);
    }
    for child in &node.children {
        let mut child_path = path.clone();
        child_path.push(child.title.as_str());
        if let Some(found) = first_pending_leaf_path(child, child_path) {
            return Some(found);
        }
    }
    None
}

/// Count of user-authored in-progress leaves (guidance target: at most one).
pub fn plan_tree_in_progress_leaf_count(tree: &PlanTree) -> usize {
    fn count(node: &PlanNode) -> usize {
        let own =
            usize::from(node.children.is_empty() && matches!(node.status, PlanStatus::InProgress));
        own + node.children.iter().map(count).sum::<usize>()
    }
    tree.roots.iter().map(count).sum()
}

/// Compact text summary for LLM context reinjection.
pub fn format_plan_tree_summary(tree: &PlanTree) -> String {
    if tree.roots.is_empty() {
        return String::new();
    }
    let mut out = String::from(PLAN_TREE_CONTEXT_PREFIX);
    out.push_str("\n\n");
    let prose = tree.prose.trim();
    if !prose.is_empty() {
        const MAX_PROSE_CHARS: usize = 2000;
        if prose.chars().count() > MAX_PROSE_CHARS {
            out.push_str(&prose.chars().take(MAX_PROSE_CHARS).collect::<String>());
            out.push_str("…\n\n");
        } else {
            out.push_str(prose);
            out.push_str("\n\n");
        }
    }
    for root in &tree.roots {
        format_node_summary(root, 0, &mut out);
    }
    if let Some(focus) = plan_tree_current_focus(tree) {
        out.push_str(&format!("Current: {focus}\n"));
    }
    if let Some(next) = plan_tree_next_pending(tree) {
        out.push_str(&format!("Next: {next}\n"));
    }
    out.trim_end().to_string()
}

fn format_node_summary(node: &PlanNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push_str(node.status.glyph());
    out.push(' ');
    out.push_str(&node.title);
    out.push_str(" (");
    out.push_str(node.id.as_str());
    out.push_str(")\n");
    for child in &node.children {
        format_node_summary(child, depth + 1, out);
    }
}

// ---------------------------------------------------------------------------
// Markdown plan document (canonical LLM-facing + storage format)
//
// A plan document is a multi-level Markdown file:
//
// ```markdown
// # 计划：重构鉴权模块
//
// 目标是 ……（指导手册式的文字说明：背景、策略、约束、验收标准）
//
// ## 计划树
//
// - [ ] 调研现状 `(research)` — 只读，不改代码
//   - [~] 阅读 auth 模块 `(read-auth)`
//   - [ ] 整理调用方 `(list-callers)`
// - [ ] 实施 `(impl)`
// ```
//
// The prose above the first `- [g]` item is the guidance section; the nested
// checkbox list below is the tree. Ids live in backticks, details after " — ".
// ---------------------------------------------------------------------------

/// Serialize a plan tree as a multi-level Markdown document.
pub fn format_plan_doc(tree: &PlanTree) -> String {
    let mut out = String::new();
    let prose = tree.prose.trim();
    if !prose.is_empty() {
        out.push_str(prose);
        out.push_str("\n\n");
    }
    for root in &tree.roots {
        format_node_doc_line(root, 0, &mut out);
    }
    out.trim_end().to_string()
}

fn format_node_doc_line(node: &PlanNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push_str("- ");
    out.push_str(node.status.glyph());
    out.push(' ');
    out.push_str(&sanitize_doc_text(&node.title));
    out.push_str(" `(");
    out.push_str(node.id.trim());
    out.push_str(")`");
    if let Some(detail) = node
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        out.push_str(" — ");
        out.push_str(&sanitize_doc_text(detail));
    }
    out.push('\n');
    for child in &node.children {
        format_node_doc_line(child, depth + 1, out);
    }
}

fn sanitize_doc_text(text: &str) -> String {
    text.replace(['\n', '\r', '`'], " ").trim().to_string()
}

struct DocLine {
    indent: usize,
    glyph: String,
    body: String,
}

/// Split a doc line into indent/glyph/body if it is a tree item.
fn parse_doc_item_line(line: &str) -> Option<DocLine> {
    let trimmed_start = line.trim_start_matches([' ', '\t']);
    let indent = line.len() - trimmed_start.len();
    let rest = trimmed_start.strip_prefix("- ")?;
    let end = rest.find(']')?;
    let glyph = rest.get(..=end)?;
    PlanStatus::from_glyph(glyph)?;
    let body = rest.get(end + 1..)?.trim().to_string();
    Some(DocLine {
        indent,
        glyph: glyph.to_string(),
        body,
    })
}

/// Parse the body after `- [g]`: `Title `(id)` — detail`.
fn parse_doc_node_body(body: &str, fallback_id: String) -> PlanNode {
    let (head, detail) = match body.split_once(" — ") {
        Some((h, d)) => (h.trim(), Some(d.trim().to_string())),
        None => (body.trim(), None),
    };
    let mut id = None;
    let mut title_parts: Vec<&str> = Vec::new();
    for part in head.split('`') {
        // Backtick-quoted segments alternate: text, `quoted`, text, …
        // Only `(slug)` shaped quoted segments are ids.
        let is_id_segment = part.starts_with('(')
            && part.ends_with(')')
            && part.len() > 2
            && !part[1..part.len() - 1].contains(char::is_whitespace);
        if id.is_none() && is_id_segment {
            id = Some(part[1..part.len() - 1].to_string());
        } else {
            title_parts.push(part);
        }
    }
    let title = title_parts.concat().trim().to_string();
    PlanNode {
        id: id.unwrap_or(fallback_id),
        title,
        status: PlanStatus::Pending,
        children: Vec::new(),
        detail: detail.filter(|d| !d.is_empty()),
        kind: None,
    }
}

/// Parse a Markdown plan document into a plan tree. Prose is everything before
/// the first tree item; nesting follows indentation (deeper indent = child).
pub fn parse_plan_doc(doc: &str) -> Result<PlanTree, PlanValidationError> {
    let mut prose_lines: Vec<&str> = Vec::new();
    let mut items: Vec<DocLine> = Vec::new();
    let mut seen_item = false;
    for line in doc.lines() {
        if let Some(item) = parse_doc_item_line(line) {
            seen_item = true;
            items.push(item);
        } else if !seen_item {
            prose_lines.push(line);
        }
        // Non-item lines after the tree started are ignored.
    }
    let prose = prose_lines.join("\n").trim().to_string();

    // Build the tree from indentation: a line is a child of the nearest
    // previous line with strictly smaller indent.
    let mut tree = PlanTree {
        prose,
        roots: Vec::new(),
    };
    // Stack of (indent, path-of-child-indices from roots).
    let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut counter = 0usize;
    for item in items {
        counter += 1;
        let status = PlanStatus::from_glyph(&item.glyph).unwrap_or_default();
        let mut node = parse_doc_node_body(&item.body, format!("node-{counter}"));
        node.status = status;
        while stack
            .last()
            .is_some_and(|(indent, _)| *indent >= item.indent)
        {
            stack.pop();
        }
        let parent_path = stack.last().map(|(_, path)| path.clone());
        let path = insert_doc_node(&mut tree, parent_path.as_deref(), node);
        stack.push((item.indent, path));
    }
    Ok(tree)
}

fn insert_doc_node(
    tree: &mut PlanTree,
    parent_path: Option<&[usize]>,
    node: PlanNode,
) -> Vec<usize> {
    fn node_at<'a>(roots: &'a mut [PlanNode], path: &[usize]) -> &'a mut PlanNode {
        let mut current = &mut roots[path[0]];
        for &idx in &path[1..] {
            current = &mut current.children[idx];
        }
        current
    }
    match parent_path {
        None => {
            tree.roots.push(node);
            vec![tree.roots.len() - 1]
        }
        Some(path) => {
            let parent = node_at(&mut tree.roots, path);
            parent.children.push(node);
            let mut child_path = path.to_vec();
            child_path.push(parent.children.len() - 1);
            child_path
        }
    }
}

/// Serialize for persistence: Markdown plan document.
pub fn plan_tree_to_storage(tree: &PlanTree) -> String {
    format_plan_doc(tree)
}

/// Deserialize from persistence. Legacy rows hold the old JSON encoding
/// (`{"roots": …}`); new rows hold the Markdown plan document.
pub fn plan_tree_from_storage(raw: &str) -> PlanTree {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') {
        serde_json::from_str(raw).unwrap_or_default()
    } else {
        parse_plan_doc(raw).unwrap_or_default()
    }
}

/// Terminal-friendly indented tree with status glyphs.
pub fn format_plan_tree_terminal(tree: &PlanTree, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let inner = width.saturating_sub(4).max(16);
    for (i, root) in tree.roots.iter().enumerate() {
        let is_last_root = i + 1 == tree.roots.len();
        format_node_terminal(root, "", is_last_root, inner, &mut lines);
    }
    lines
}

fn format_node_terminal(
    node: &PlanNode,
    prefix: &str,
    is_last: bool,
    width: usize,
    lines: &mut Vec<String>,
) {
    let branch = if is_last { "└─ " } else { "├─ " };
    let cont = if is_last { "   " } else { "│  " };
    let title = truncate_chars(
        &node.title,
        width.saturating_sub(prefix.len() + branch.len() + 6),
    );
    let line = format!(
        "{}{}{} {} {}",
        prefix,
        branch,
        node.status.glyph(),
        title,
        node.id
    );
    lines.push(line);
    let child_prefix = format!("{}{}", prefix, cont);
    for (i, child) in node.children.iter().enumerate() {
        let last = i + 1 == node.children.len();
        format_node_terminal(child, &child_prefix, last, width, lines);
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!(
        "{}…",
        s.chars().take(max.saturating_sub(1)).collect::<String>()
    )
}

pub fn plan_tree_is_empty(tree: &PlanTree) -> bool {
    tree.roots.is_empty()
}

pub fn plan_tree_all_completed(tree: &PlanTree) -> bool {
    !tree.roots.is_empty() && tree.roots.iter().all(subtree_all_completed)
}

fn subtree_all_completed(node: &PlanNode) -> bool {
    matches!(node.status, PlanStatus::Completed) && node.children.iter().all(subtree_all_completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> PlanTree {
        PlanTree {
            prose: String::new(),
            roots: vec![PlanNode {
                id: "phase-1".into(),
                title: "Research".into(),
                status: PlanStatus::Pending,
                children: vec![PlanNode {
                    id: "task-1".into(),
                    title: "Read code".into(),
                    status: PlanStatus::Completed,
                    children: vec![],
                    detail: None,
                    kind: Some(PlanNodeKind::Task),
                }],
                detail: None,
                kind: Some(PlanNodeKind::Phase),
            }],
        }
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let tree = PlanTree {
            prose: String::new(),
            roots: vec![
                PlanNode {
                    id: "a".into(),
                    title: "A".into(),
                    status: PlanStatus::Pending,
                    children: vec![],
                    detail: None,
                    kind: None,
                },
                PlanNode {
                    id: "a".into(),
                    title: "B".into(),
                    status: PlanStatus::Pending,
                    children: vec![],
                    detail: None,
                    kind: None,
                },
            ],
        };
        let err = validate_plan_tree(&tree, &PlanLimits::default_mvp()).unwrap_err();
        assert!(err.message.contains("duplicate"));
    }

    #[test]
    fn rollup_marks_parent_completed() {
        let mut tree = sample_tree();
        tree.roots[0].children[0].status = PlanStatus::Completed;
        rollup_plan_statuses(&mut tree);
        assert_eq!(tree.roots[0].status, PlanStatus::Completed);
    }

    #[test]
    fn apply_patch_updates_status() {
        let mut tree = sample_tree();
        apply_plan_patches(
            &mut tree,
            &[PlanPatch {
                id: Some("task-1".into()),
                parent_id: None,
                status: Some("in_progress".into()),
                title: None,
                detail: None,
                node: None,
            }],
        )
        .unwrap();
        assert_eq!(tree.roots[0].children[0].status, PlanStatus::InProgress);
    }

    #[test]
    fn summary_and_terminal_formats() {
        let tree = sample_tree();
        let summary = format_plan_tree_summary(&tree);
        assert!(summary.contains("## Active Plan Tree"));
        assert!(summary.contains("Research"));
        let lines = format_plan_tree_terminal(&tree, 80);
        assert!(!lines.is_empty());
        assert!(lines[0].contains("Research"));
    }

    #[test]
    fn focus_and_next_guide_lines() {
        let mut tree = PlanTree {
            prose: String::new(),
            roots: vec![PlanNode {
                id: "phase-1".into(),
                title: "Build".into(),
                status: PlanStatus::Pending,
                children: vec![
                    PlanNode {
                        id: "t1".into(),
                        title: "Read code".into(),
                        status: PlanStatus::InProgress,
                        children: vec![],
                        detail: None,
                        kind: None,
                    },
                    PlanNode {
                        id: "t2".into(),
                        title: "Write tests".into(),
                        status: PlanStatus::Pending,
                        children: vec![],
                        detail: None,
                        kind: None,
                    },
                ],
                detail: None,
                kind: None,
            }],
        };
        rollup_plan_statuses(&mut tree);
        assert_eq!(
            plan_tree_current_focus(&tree).as_deref(),
            Some("Build / Read code")
        );
        assert_eq!(
            plan_tree_next_pending(&tree).as_deref(),
            Some("Build / Write tests")
        );
        assert_eq!(plan_tree_in_progress_leaf_count(&tree), 1);
        let summary = format_plan_tree_summary(&tree);
        assert!(summary.contains("Current: Build / Read code"));
        assert!(summary.contains("Next: Build / Write tests"));
    }

    #[test]
    fn focus_prefers_deepest_in_progress() {
        let tree = PlanTree {
            prose: String::new(),
            roots: vec![PlanNode {
                id: "p".into(),
                title: "Phase".into(),
                status: PlanStatus::InProgress,
                children: vec![PlanNode {
                    id: "t".into(),
                    title: "Leaf task".into(),
                    status: PlanStatus::InProgress,
                    children: vec![],
                    detail: None,
                    kind: None,
                }],
                detail: None,
                kind: None,
            }],
        };
        assert_eq!(
            plan_tree_current_focus(&tree).as_deref(),
            Some("Phase / Leaf task")
        );
        assert_eq!(plan_tree_in_progress_leaf_count(&tree), 1);
    }

    #[test]
    fn plan_doc_round_trip() {
        let doc = "# 计划：重构鉴权模块\n\n目标是拆出 token 校验；先调研再动手，改完跑全量测试。\n\n## 计划树\n\n- [ ] 调研现状 `(research)` — 只读，不改代码\n  - [~] 阅读 auth 模块 `(read-auth)`\n  - [ ] 整理调用方 `(list-callers)`\n- [ ] 实施 `(impl)`\n  - [ ] 拆分校验层 `(split)`\n    - [X] 失败的尝试 `(failed-try)`\n";
        let tree = parse_plan_doc(doc).unwrap();
        assert!(tree.prose.contains("目标是拆出 token 校验"));
        assert!(tree.prose.contains("# 计划：重构鉴权模块"));
        assert_eq!(tree.roots.len(), 2);
        let research = &tree.roots[0];
        assert_eq!(research.id, "research");
        assert_eq!(research.title, "调研现状");
        assert_eq!(research.detail.as_deref(), Some("只读，不改代码"));
        assert_eq!(research.status, PlanStatus::Pending);
        assert_eq!(research.children.len(), 2);
        assert_eq!(research.children[0].id, "read-auth");
        assert_eq!(research.children[0].status, PlanStatus::InProgress);
        let split = &tree.roots[1].children[0];
        assert_eq!(split.id, "split");
        assert_eq!(split.children[0].status, PlanStatus::Failed);

        let rendered = format_plan_doc(&tree);
        let reparsed = parse_plan_doc(&rendered).unwrap();
        assert_eq!(reparsed.prose, tree.prose);
        assert_eq!(reparsed.roots.len(), 2);
        assert_eq!(reparsed.roots[0].children[1].id, "list-callers");
        assert_eq!(reparsed.roots[1].children[0].children[0].id, "failed-try");
    }

    #[test]
    fn plan_doc_assigns_fallback_ids_and_glyphs() {
        let doc = "- [x] done thing\n  - [!] blocked thing\n    - [-] dropped thing\n";
        let tree = parse_plan_doc(doc).unwrap();
        assert_eq!(tree.roots[0].status, PlanStatus::Completed);
        assert!(!tree.roots[0].id.is_empty());
        assert_eq!(tree.roots[0].children[0].status, PlanStatus::Blocked);
        assert_eq!(
            tree.roots[0].children[0].children[0].status,
            PlanStatus::Cancelled
        );
        assert!(tree.prose.is_empty());
    }

    #[test]
    fn storage_round_trip_and_legacy_json() {
        let mut tree = PlanTree {
            prose: "指导说明：稳步推进。".into(),
            roots: vec![PlanNode {
                id: "a".into(),
                title: "Task A".into(),
                status: PlanStatus::InProgress,
                children: vec![],
                detail: None,
                kind: None,
            }],
        };
        let stored = plan_tree_to_storage(&tree);
        assert!(stored.contains("- [~] Task A `(a)`"));
        let loaded = plan_tree_from_storage(&stored);
        assert_eq!(loaded.prose, tree.prose);
        assert_eq!(loaded.roots[0].id, "a");
        assert_eq!(loaded.roots[0].status, PlanStatus::InProgress);

        // Legacy JSON rows still load.
        let legacy = serde_json::to_string(&tree).unwrap();
        let loaded_legacy = plan_tree_from_storage(&legacy);
        assert_eq!(loaded_legacy.roots[0].id, "a");
        assert_eq!(loaded_legacy.prose, "指导说明：稳步推进。");

        tree.prose.clear();
        tree.roots.clear();
        assert!(plan_tree_to_storage(&tree).is_empty());
    }

    #[test]
    fn summary_includes_prose() {
        let tree = PlanTree {
            prose: "先读代码，再小步修改。".into(),
            roots: vec![PlanNode {
                id: "a".into(),
                title: "Work".into(),
                status: PlanStatus::Pending,
                children: vec![],
                detail: None,
                kind: None,
            }],
        };
        let summary = format_plan_tree_summary(&tree);
        assert!(summary.contains("先读代码，再小步修改。"));
        assert!(summary.contains("[ ] Work (a)"));
    }
}
