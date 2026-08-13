//! Composer modes — per-task behavior overrides selected via `/` slash commands.
//!
//! Modes are task-scoped: they ride on the `composer_mode` field of a single
//! message (or a session's composer state) and never change the session's
//! persisted agent / global routing.

/// "Grill me / 拷问" mode — Socratic plan alignment before implementation.
pub const GRILL_COMPOSER_MODE: &str = "grill";
/// "Plan / 计划" mode — read-only exploration that ends in a plan document.
pub const PLAN_COMPOSER_MODE: &str = "plan";

pub fn normalize_composer_mode(raw: Option<&str>) -> Option<&'static str> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(m) if m.eq_ignore_ascii_case(GRILL_COMPOSER_MODE) => Some(GRILL_COMPOSER_MODE),
        Some(m) if m.eq_ignore_ascii_case(PLAN_COMPOSER_MODE) => Some(PLAN_COMPOSER_MODE),
        _ => None,
    }
}

pub fn system_append_for_mode(mode: Option<&str>, reply_lang: &str) -> Option<&'static str> {
    match normalize_composer_mode(mode) {
        Some(GRILL_COMPOSER_MODE) => Some(grill_system_append(reply_lang)),
        Some(PLAN_COMPOSER_MODE) => Some(plan_system_append(reply_lang)),
        _ => None,
    }
}

/// Tools blocked while a composer mode is active.
pub fn tool_deny_names_for_mode(mode: Option<&str>) -> &'static [&'static str] {
    match normalize_composer_mode(mode) {
        Some(GRILL_COMPOSER_MODE) => grill_tool_deny_names(),
        Some(PLAN_COMPOSER_MODE) => plan_tool_deny_names(),
        _ => &[],
    }
}

pub fn grill_system_append(reply_lang: &str) -> &'static str {
    if reply_lang.starts_with("zh") {
        GRILL_APPEND_ZH
    } else {
        GRILL_APPEND_EN
    }
}

pub fn plan_system_append(reply_lang: &str) -> &'static str {
    if reply_lang.starts_with("zh") {
        PLAN_APPEND_ZH
    } else {
        PLAN_APPEND_EN
    }
}

/// Tools blocked while composer is in grill mode (alignment before implementation).
pub fn grill_tool_deny_names() -> &'static [&'static str] {
    &[
        "Bash",
        "Edit",
        "Write",
        "FileWrite",
        "ApplyPatch",
        "NotebookEdit",
        "PowerShell",
    ]
}

/// Tools blocked in plan mode: file mutation is off-limits; read-only tools and
/// read-only Bash exploration stay available so the plan can be grounded in the
/// actual codebase.
pub fn plan_tool_deny_names() -> &'static [&'static str] {
    &[
        "Edit",
        "Write",
        "FileWrite",
        "ApplyPatch",
        "NotebookEdit",
        "PowerShell",
    ]
}

const GRILL_APPEND_ZH: &str = r#"## 拷问模式（Grill Me）

用户已进入 **拷问模式**：在双方达成共同理解、且用户明确允许动手之前，**禁止写代码、改文件、跑破坏性命令**。

### 流程
1. **一次只问一个问题**。必须用 `AskUserQuestion` 工具提问（不要用纯 Markdown 列表一次性抛出多个问题）。
2. **每个问题都要给出推荐答案**：把最可能正确的选项放在第一项，标签含「（推荐）」；其余 2–4 个选项覆盖常见分歧。
3. **能自己查的就别问用户**：仓库结构、已有命令、配置位置、API 路由、技能/工具能力等——先用 Read/Grep/Glob 查代码库，不要把能在代码里找到答案的问题抛给用户。
4. **等用户答完再问下一题**。收到回答后简短确认，再进入下一维度。
5. **退出**：当用户说「可以动手了」「开始实现」等，或选项里明确「理解已对齐，开始实现」时——用 3–5 条 bullet **复述共识**（目标、范围、验收、不做项），然后停止追问，等待用户下一条实施指令。

### 拷问维度（按 relevance 选，不必全问）
- 目标与成功标准（做完怎么算对）
- 范围边界（做什么 / 不做什么）
- 用户角色与交付物形态
- 约束（时间、环境、不能动的部分）
- 风险与回滚

### 语气
直接、具体、无套话；不要 emoji。"#;

const GRILL_APPEND_EN: &str = r#"## Grill Me mode

The user enabled **Grill Me**: do **not** write code, edit files, or run destructive commands until you both align and the user explicitly allows implementation.

### Protocol
1. **One question at a time**. Always use the `AskUserQuestion` tool (never dump multiple questions in Markdown).
2. **Every question includes a recommended answer**: put the best guess first with "(Recommended)" in the label; offer 2–4 other plausible options.
3. **Answer from the repo yourself**: layout, commands, config paths, APIs, skills/tools — use Read/Grep/Glob before asking the user anything you could infer from code.
4. **Wait for the user's reply** before the next question. Briefly acknowledge each answer.
5. **Exit**: When the user says "go ahead", "start implementing", or picks an option that means alignment — summarize consensus in 3–5 bullets (goal, scope, acceptance, out-of-scope), then stop grilling.

### Dimensions (pick what matters; don't exhaust a checklist)
- Goal and definition of done
- Scope in / out
- Audience and deliverable shape
- Constraints (env, time, must-not-touch)
- Risks and rollback

### Tone
Direct, specific, no filler; no emoji."#;

const PLAN_APPEND_ZH: &str = r#"## 计划模式（Plan Mode）

用户已进入 **计划模式**：这是一个只读任务——**禁止修改任何文件、禁止运行有副作用的命令**（写文件、删除、安装、改 git 状态等）。只读探索（Read/Grep/Glob、只读的 Bash 如 `ls`/`git log`/`cargo metadata`）是允许的，且应该充分使用。

### 目标
产出一份**可执行的计划文档**，而不是直接动手。流程：

1. **先探索**：围绕用户需求读代码、查结构、确认关键文件与约束。不确定的重大分歧用 `AskUserQuestion` 澄清。
2. **写计划**：用 `PlanWrite` 工具的 `doc` 字段输出一份多层级 Markdown 计划文档：
   - **顶部是指导说明**（像一份简短的实施手册）：目标、背景、策略、约束、验收标准；
   - **底部是多层级的详细计划树**：嵌套复选框列表，`- [ ] 标题 `(id)` — 细节`，子级缩进 2 空格，id 用稳定的短 slug。
3. **呈现计划**：在回复中概述计划要点，提示用户在 Plan 面板审阅并点击 Build 后才会开始执行。

### 边界
- 计划要具体：每个叶子节点指向明确的文件/模块/动作，避免「优化代码」这种空泛条目。
- 不要执行计划里的任何一步；用户确认后会以新指令触发实施。
- 语气直接、无套话。"#;

const PLAN_APPEND_EN: &str = r#"## Plan Mode

The user enabled **Plan Mode**: this is a read-only task — do **not** modify any files or run commands with side effects (writes, deletes, installs, git mutations). Read-only exploration (Read/Grep/Glob, read-only Bash like `ls`/`git log`) is allowed and expected.

### Goal
Produce an **executable plan document** instead of implementing. Protocol:

1. **Explore first**: read code, map structure, confirm key files and constraints. Use `AskUserQuestion` for genuinely ambiguous forks.
2. **Write the plan**: call `PlanWrite` with the `doc` field — a multi-level Markdown plan document:
   - **Top: guidance prose** (a short instruction manual): goal, background, strategy, constraints, acceptance criteria;
   - **Bottom: the detailed multi-level plan tree**: a nested checkbox list, `- [ ] Title `(id)` — detail`, children indented by 2 spaces, ids are stable short slugs.
3. **Present the plan**: summarize the key points in your reply and tell the user to review it in the Plan panel — execution starts only after they confirm (Build).

### Boundaries
- Be concrete: every leaf names a specific file/module/action; avoid vague items like "improve code".
- Do not execute any step of the plan; implementation is triggered by a follow-up instruction after user confirmation.
- Direct tone, no filler."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_grill_mode() {
        assert_eq!(
            normalize_composer_mode(Some("grill")),
            Some(GRILL_COMPOSER_MODE)
        );
        assert_eq!(
            normalize_composer_mode(Some(" GRILL ")),
            Some(GRILL_COMPOSER_MODE)
        );
        assert_eq!(
            normalize_composer_mode(Some("plan")),
            Some(PLAN_COMPOSER_MODE)
        );
        assert_eq!(
            normalize_composer_mode(Some(" Plan ")),
            Some(PLAN_COMPOSER_MODE)
        );
        assert_eq!(normalize_composer_mode(Some("other")), None);
        assert_eq!(normalize_composer_mode(None), None);
    }

    #[test]
    fn append_for_each_mode() {
        assert!(system_append_for_mode(Some("grill"), "zh").is_some());
        assert!(system_append_for_mode(Some("plan"), "zh").is_some());
        assert!(system_append_for_mode(Some("plan"), "en").is_some());
        assert!(system_append_for_mode(Some("other"), "en").is_none());
    }

    #[test]
    fn grill_tool_deny_blocks_implementation_tools() {
        let denied = grill_tool_deny_names();
        assert!(denied.contains(&"Bash"));
        assert!(denied.contains(&"Edit"));
        assert!(!denied.contains(&"AskUserQuestion"));
    }

    #[test]
    fn plan_tool_deny_keeps_readonly_bash() {
        let denied = tool_deny_names_for_mode(Some("plan"));
        assert!(denied.contains(&"Edit"));
        assert!(denied.contains(&"Write"));
        // Plan mode keeps Bash for read-only exploration.
        assert!(!denied.contains(&"Bash"));
        assert!(!denied.contains(&"PlanWrite"));
        assert!(tool_deny_names_for_mode(None).is_empty());
        assert_eq!(
            tool_deny_names_for_mode(Some("grill")).len(),
            grill_tool_deny_names().len()
        );
    }
}
