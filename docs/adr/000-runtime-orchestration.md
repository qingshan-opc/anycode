# ADR 000: AgentRuntime as the sole orchestration authority

## Status

Accepted (current)

## Context

anyCode exposes an **`Agent`** trait in `anycode-core` with an **`execute`** method, while the CLI and TUI (and other supported entrypoints such as **`run`**) actually run tasks through **`AgentRuntime::execute_task`** and **`execute_turn_from_messages`**. Contributors can assume the wrong entry point unless the rule is documented.

Multi-step workflow DAGs (Workbench graph runs, cron orchestration) need a coordinator that is **not** a second ReAct loop.

## Decision

1. **Orchestration authority**: Multi-turn LLM calls, tool execution, logging, and (where applicable) summary generation are implemented **only** in **`anycode_agent::AgentRuntime`** (`execute_task` for one-shot tasks, `execute_turn_from_messages` for TUI sessions).
2. **`Agent` trait role**: Supplies **agent type**, **tool name subset** (`tools()`), **description**, and **system prompt** hooks. The default **`Agent::execute`** implementations are **not** invoked by the current CLI/TUI main paths.
3. **GraphEngine (multi-step DAG)**: **`anycode_agent::GraphEngine`** coordinates multi-step workflow DAGs — topological scheduling, checkpoints, retries, and step handoff. Each step still runs through **`AgentRuntime::execute_task`** (single-step ReAct). Workbench calls GraphEngine via `POST /api/sessions/{id}/graph/run`; cron/scheduler may use richer wrappers in channel-bridge.
4. **Extensions**: New capabilities should extend **`Tool`** + **`build_registry_with_services`** + CLI **`bootstrap`**, not a second parallel “runner” trait hierarchy.

## Consequences

- Documentation and onboarding must point to **`AgentRuntime`** first; see `crates/agent/README.md` and [user architecture guide](https://anycode.work/docs/guide/architecture).
- Graph/workflow features must not re-implement tool loops inside dashboard handlers — delegate to **`GraphEngine::run`**.
- If a future mode truly needs **`Agent::execute`**, that should be a deliberate ADR amendment with call sites listed.

## Related

- `anycode-core`: `Agent` trait rustdoc
- `crates/agent/src/runtime/mod.rs`
- `crates/agent/src/graph_engine.rs`
- `https://anycode.work/docs/guide/architecture`
