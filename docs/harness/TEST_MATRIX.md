# Required test matrix

Execution status belongs in INTEGRATION_20260918.md and the referenced CI logs; this matrix describes coverage and remaining real integration work.

| Area | Included code tests | Real integration still required |
|---|---|---|
| Kernel | tool order, duplicate IDs, capability deny, uncertain output, budget, completion hook, follow-up | existing provider streams, failover, compaction, session and UI cancellation |
| Graph | DAG validation, Partial, gate failure, Human revision, scope/definition mismatch, uncertain recovery, conditional any join | actual NodeExecutor profiles/verifiers, process crash and live storage |
| Subagent | shared budget, separate root rejection, lineage/cancel in context tests | multi-profile write isolation, shared old services migration, native process cleanup |
| Computer | frame/action-bound approval, exclusive lease, unsupported keys, device mismatch | X11 real display, focus race handling; macOS/Windows native drivers |
| 818cloud | issuer/audience/context/origin shape | PostgreSQL/Redis, grant revoke, project membership, device credential lifecycle |
| Frontend | 52 node:test cases plus existing Dashboard suite | mounted ReactFlow product page, auth callbacks, graph E2E |
| Installer | 21 unittest checks in the separately delivered package | source integration now handled by Git commits; do not reapply old patches |

Rust test source being present is not execution evidence. A passing scripted fixture is not a passing live-model/desktop/cloud test.
