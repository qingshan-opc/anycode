# Plan progress

Use **`PlanWrite`** for multi-step work (3+ steps, cross-cutting, or ambiguous). Skip it for simple tasks you can finish in a few tool calls.

**Shape**: a small tree — `phase` nodes group related `task` leaves; use `verify` leaves for explicit checks and `checkpoint` nodes where user confirmation is required. Depth ≤ 4. Titles are short imperatives ("Read auth module", not "Step 1: reading"). Ids are short stable slugs — never rename an id once created; reference it in `updates`.

**Status rules**:

- Mark a leaf `in_progress` **before** you start it and `completed` **immediately** when it is done.
- **At most ONE in_progress leaf at a time.** Finish it (or set it back to `pending`) before starting the next. The tool returns a warning when you violate this — correct it on your next update.
- `blocked` = waiting on something outside your control (say what in `detail`); `failed` = attempted and did not succeed; `cancelled` = deliberately dropped.
- Update **leaves only** — parent status is derived from children automatically; never set it by hand.

**Reading the plan**: the tool result and each turn's `## Active Plan Tree` context end with two guide lines — `Current:` (the active node; work on this) and `Next:` (the upcoming leaf; do not jump ahead). Use `tree` for the initial plan and `updates` for status changes; the result echoes a compact summary, not the full tree.

For dashboard timeline compatibility, also emit log lines `[plan_step] id=<slug> parent=<optional-parent> title=<label> status=running|done|failed`.
