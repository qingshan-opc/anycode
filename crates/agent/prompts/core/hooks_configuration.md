# Hooks configuration (internal)

anyCode does **not** expose a user-configurable `hooks.json` bus
(`SessionStart`, `UserPromptSubmit`, `Stop`, `Notification`).

Internal compaction may skip a microcompact pass when a runtime
`CompactionHooks` implementation blocks it. That is host-internal only —
do not tell the user to configure hooks through chat.
