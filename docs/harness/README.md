# Harness integration: foundation, not a production cutover

This is an independent Rust implementation inspired by Pi's small execution kernel and host/extension boundaries. It does not copy the upstream TypeScript runtime or claim compatibility with Pi extensions, LangGraph Python, or the Grapl security platform.

Baseline: `0411ea3a94fe6aa2d37326f9342cfc5d6a13f4aa`. Review history: PR #1. Normalized integration: `b426a7e66b39241ab0936de4b2ee39ebba9ac4e9`.

## Wired source

Four workspace members: `harness-core`, `harness-extensions`, `harness-host`, `harness-cloud818`. `anycode-agent` exposes the existing security-chain adapter only under the opt-in `harness-v1` feature. Default Workbench, scheduler and existing graph routes are unchanged. Workspace inclusion does not mean runtime migration is complete.

The core owns one LLM/tool loop, native message preservation, scoped authority, shared parent/child budgets, cancellation, action-bound approvals, events and session trees. Hosts supply checked execution, context projection and completion validation.

Extensions coordinate a bounded DAG, explicit checkpoints, evidence gates, human decisions, children, worktrees, skills and computer leases. Work nodes and children use the same kernel. Partial or uncertain work is not success. Interrupted side effects need reconciliation instead of automatic repetition.

The native host calls the existing checked tool invocation chain, not bare `Tool::execute`. The local FileRead pilot is not an enterprise authorization implementation.

The cloud adapter uses Accounts' existing SSO v2 introspection contract server-side. Login does not replace product/project ACL checks. Never distribute confidential client credentials in Tauri or browser code. No second wallet or production account migration is introduced.

The ReactFlow component and validators are under `crates/dashboard-ui/src/features/harness`. The component compiles; product routing and authenticated start/resume/approval callbacks are not automatically connected. Node regressions are explicitly in `npm test`; the old Vitest rule did not collect `.test.mjs` files.

## Verified evidence and limitations

Actions run `35261253665` checked out `b426a7e6` and passed all four new crates' tests, strict Clippy, the scripted fixture, and `cargo check --locked -p anycode-agent --features harness-v1`. The UI job passed `npm ci`, 441 original Vitest tests, 52 Node regressions, and `tsc -b && vite build`.

The scripted fixture and computer fake are contract tests, not live-model or real-desktop tests. The user's Cloud Mac command was refused at the conversation layer; no file or terminal was changed there. GitHub-hosted macOS tests are separate from the user's Mac.

Initial existing account-service CI exposed unused `DEFAULT_WINDOW_SECS` in non-test builds. The follow-up marks this test fixture `cfg(test)`, retaining its assertion and all database-driven runtime quota behavior. No lint gate, database schema or billing behavior is changed. The latest PR jobs, not earlier commits, determine final candidate validation.

The existing npm install reported 12 dependency advisories (7 moderate, 4 high, 1 critical). Neither dependency specifications nor the npm lock were changed here; remediation requires a separate reviewed update, not a force-upgrade.

The temporary write-capable normalizer is removed. `harness.yml` is read-only and tests Linux/macOS foundations, optional Agent compilation, graph regressions and examples. Existing CI remains intact.

## Reproduce

```sh
python3 tools/harness/verify.py --mode node
python3 tools/harness/verify.py --mode integrated-rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd crates/dashboard-ui && npm ci && npm test && npm run build)
```

`standalone-rust` resolves a temporary workspace without the native host; it cannot replace integrated locked checks. Missing tools return nonzero.

## Next stages

See `CURSOR_START.md`. Native macOS/Windows/Wayland computer backends, live-model approval/stream recovery, authenticated graph routes, original lifecycle parity, desktop pairing and live Accounts/project ACL integration remain. Preserve existing account and wallet stores. Source integration is not a production deployment or public release.
