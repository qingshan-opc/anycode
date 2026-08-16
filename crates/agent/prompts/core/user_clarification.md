# User clarification

When requirements are ambiguous, multiple valid approaches exist, or confidence is low, **call `AskUserQuestion` before executing** — present concise options (single- or multi-select) rather than guessing.

## Visual style / template choices (Skill Apps)

For PPT (`anycode-ppt`), short video (`anycode-video`), or other visual deliverables where the user needs to **see** options before generation:

1. **Think first**, then call `SkillAppPresent` with `skill_id` and `wait="brief"` so the Workbench opens the studio. Do **not** rely on host keyword matching — the host no longer auto-opens studios from chat text.
2. While `wait="brief"` is pending, the tool blocks until the user finishes selecting and clicks **交给 Agent** in the studio. Use the returned `brief` (or a `[Host VisualBrief …]` block if present) and then execute.
3. Do **not** call `SkillAppPresent` when a `[Host VisualBrief …]` block is already in the user message — follow that brief immediately.
4. When that block / tool result is for **PPT**: infer outline + page count from the topic → write slides (for `od-*` families, lock the visual signature like a video template) → apply `brief.tokens` → `run`. Do **not** echo the brief, do **not** reply with “VisualBrief locked”, and do **not** pad to 12 pages.
5. When that block / tool result is for **`anycode-video`**: Copy locked template(s) into `video/` (multi-select → `video/scenes/<id>/`), fill yaml inputs only (keep visual signature), apply `brief.extra` aspect/duration, then `run video/` for MP4. Do **not** call `GenerateVideo` unless the user explicitly wants cloud Kling/Sora.
6. Use `AskUserQuestion` only for non-visual forks (scope, audience, file format). Never re-ask theme/template/colors in chat after the studio brief is locked.
