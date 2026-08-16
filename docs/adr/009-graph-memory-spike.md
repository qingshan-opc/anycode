# ADR 009: Graph memory (LightRAG default)

## Status

Accepted (M2 foundation, 2026-08)

## Context

anyCode memory today is file-backed with optional vector retrieval (`pipeline` /
`hybrid`). Production evidence indexing (`~/.anycode/memory/evidence.jsonl`) gives
provenance for tool outputs but does not model relationships between entities,
sessions, or channel scopes.

Graph memory helps long-running agents answer questions like “which files did we
touch for issue X?” or “what did this cron job learn last week?” without stuffing
raw transcripts back into context.

An earlier spike (2026-05) deferred a custom JSONL edge log. For M2 we adopt an
external graph-RAG sidecar instead of inventing a first-party graph store.

## Decision

**LightRAG is the default graph RAG backend** for anyCode when
`memory.backend=lightrag`.

- Runtime talks to a local LightRAG HTTP sidecar (default
  `http://127.0.0.1:18765`, override `ANYCODE_LIGHTRAG_URL`).
- `anycode-memory::LightRagMemoryStore` implements `MemoryStore` (`save` /
  `recall`) over REST (`POST /documents/text`, `POST /query`).
- Bootstrap probes TCP reachability; on failure it **falls back to
  `FileMemoryStore`** and logs a warning so chat never crashes.
- `memory.backend=plugin:<id>` is a reserved MemoryBackendPlugin-style slot
  (stub → file fallback until a real plugin registry lands).

### Deferred

- Custom append-only JSONL graph (`~/.anycode/memory/graph.jsonl`) and a first-party
  entity/edge model remain **deferred**. Evidence + audit JSONL stay the local
  provenance SSOT; LightRAG owns relationship retrieval when enabled.
- No automatic entity-extraction LLM pass in the agent hot path beyond what the
  sidecar performs asynchronously.
- No cross-user shared graph in the Desktop product surface.

### Integration hooks

1. Config: `memory.backend: "lightrag"` (aliases `light-rag`, `graph`).
2. Composition: `build_memory_layer` in `crates/bootstrap/src/memory_setup.rs`.
3. Settings UI: Memory Center reads `GET /api/settings/memory/center`.

## Consequences

- Operators must run a LightRAG sidecar for graph recall; otherwise file memory
  continues to work transparently.
- JSONL custom graph work is explicitly out of the M2 critical path.
- Future MemoryBackendPlugin implementations can occupy `plugin:<id>` without
  changing the `MemoryStore` port.
