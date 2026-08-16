# OpenAPI freeze (M5 stub)

## Intent

Generate a checked-in OpenAPI document for the Workbench HTTP API and derive TypeScript client types from it. Full coverage (~100 routes) is deferred; this note records the target and interim approach.

## Current state

- No `utoipa` / OpenAPI generator in the Rust workspace today.
- Hand-maintained fetch helpers live under `crates/dashboard-ui/src/api/client/`.
- Canonical session transcript blocks already come from persisted `chat_turn_events` (`GET /api/sessions/{id}/transcript` prefers that path).

## Near-term plan

1. Add `utoipa` to `anycode-dashboard` for a **minimal** spec first: `GET /health`, `GET /api/sessions/{id}`, `GET /api/sessions/{id}/transcript`, `GET /api/settings/security`.
2. Check in `docs/ops/openapi/workbench-minimal.json` from that subset.
3. Optional: `openapi-typescript` job in `dashboard-ui` CI once the minimal spec is stable.
4. Expand route coverage incrementally by handler module (settings → sessions → chat SSE metadata).

## Out of scope (for now)

- One-shot generation of all Workbench routes before handler contracts stabilize.
- Replacing every existing client module in a single PR.
