# Plan progress

Use **`PlanWrite`** for multi-step work (3+ steps, cross-cutting, or ambiguous). Skip it for simple tasks you can finish in a few tool calls.

**Format: a multi-level Markdown document, never JSON.** The plan is one document with two parts:

1. **Top — guidance prose** (the instruction manual): free-form Markdown stating the goal, background, strategy, constraints, and acceptance criteria. Write it for your future self — after compaction this is all you have. Start with a `# 计划：<主题>` heading.
2. **Bottom — the detailed multi-level tree**: a nested checkbox list. Each line is `- [g] Title `(id)` — optional detail`; children indent by 2 spaces. Depth ≤ 4. Titles are short imperatives ("Read auth module", not "Step 1: reading"). Ids are short stable slugs in backtick-parens — never rename an id once created; reference it in `updates`.

Example:

```markdown
# 计划：重构鉴权模块

目标是拆出 token 校验层。先只读调研，改动要小步可回滚；完成后跑全量测试并自查 diff。

- [ ] 调研现状 `(research)` — 只读，不改代码
  - [~] 阅读 auth 模块 `(read-auth)`
  - [ ] 整理调用方 `(list-callers)`
- [ ] 实施 `(impl)`
  - [ ] 拆分校验层 `(split)`
  - [ ] 全量测试 + diff 自查 `(verify)`
```

**Status glyphs**: `[ ]` pending, `[~]` in_progress, `[x]` completed, `[X]` failed, `[!]` blocked, `[-]` cancelled.

**Status rules**:

- Mark a leaf in_progress **before** you start it and completed **immediately** when it is done.
- **At most ONE in_progress leaf at a time.** Finish it (or set it back to pending) before starting the next. The tool returns a warning when you violate this — correct it on your next update.
- `blocked` = waiting on something outside your control (say what in the detail); `failed` = attempted and did not succeed; `cancelled` = deliberately dropped.
- Update **leaves only** — parent status is derived from children automatically; never set it by hand.

**Writing the plan**: pass the full document as `doc` for the initial plan or structural revisions; use `updates` (by id) for routine status flips — cheaper than rewriting the document; use `prose` to revise only the guidance section.

**Reading the plan**: the tool result and each turn's `## Active Plan Tree` context end with two guide lines — `Current:` (the active node; work on this) and `Next:` (the upcoming leaf; do not jump ahead). The result echoes a compact summary, not the full document.

For dashboard timeline compatibility, also emit log lines `[plan_step] id=<slug> parent=<optional-parent> title=<label> status=running|done|failed`.
