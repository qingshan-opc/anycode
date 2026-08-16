# ADR 020: Skill Apps — skill-bound visual workbench

- Status: Accepted (2026-08-13)
- Relates: ADR 008 (AskUserQuestion host), ADR 015 (commercial office), skills catalog

## Context

Skills (`SKILL.md` + optional `run`) solved Agent scaffolding. Human–agent
collaboration stayed text-only: `AskUserQuestion` is label/description options;
style selection for PPT/Doc is verbal or post-hoc deliverable preview. Users
cannot lock a visual brief *before* generation.

The workbench already has dock panels, conversation-area tabs, and project
sidebar groups. What is missing is a **skill-authored, sandboxed UI** that can
mount in those slots and exchange structured state with the Agent.

## Decision

### Skill App

A skill may ship a `ui/` directory:

```text
my-skill/
  SKILL.md
  ui/
    surface.yaml   # host declaration
    index.html     # entry
    …
```

`SKILL.md` frontmatter may reference `ui: surface.yaml` (default when
`ui/surface.yaml` exists). The host loads the HTML in a **sandboxed iframe**
(distinct origin; `sandbox="allow-scripts"` without `allow-same-origin`).

### Host slots

| Slot | Surface |
|------|---------|
| `dock` | Workbench header icons + right dock (same chrome as Files/Browser) |
| `conversation` | Main-area tab beside Chat (`moveToConversationTab`) |
| `project` | Pin under the project group in the session sidebar |

Bindings are project-scoped (`project_skill_apps` + optional
`.anycode/skill-apps.json`). Skill declares `slots` + `default_slot`.

Project-pinned apps prefer the **conversation** slot when opened, so expanding
the dock (which collapses the left sidebar) does not hide the pin affordance.

### Data plane

- **VisualBrief** — user-locked structured contract (theme/templates/tokens).
  When present, skill SOPs must follow it; no verbal style guessing.
- **SkillAppState** — mini-app runtime state, keyed by `(project_id, skill_id)`
  when `lifecycle: persistent`.
- Agent tools: `SkillAppPresent` (optional `wait: brief`), `SkillAppPush`,
  `SkillAppRead`.
- Bridge: postMessage JSON-RPC (`anycode.*` Host SDK injected by the workbench).
  Skills must not call dashboard REST directly.

### Present lifecycle (HITL round)

Each style-pick is one present cycle. UI is **not** kept open while the Agent runs.

```text
idle → presented(wait_brief) → locked → dismissed
```

1. **presented** — Model calls `SkillAppPresent` with `wait: brief` (LLM decides which studio). Workbench opens the conversation tab and mounts the sandboxed iframe. The host does **not** keyword-match chat text to auto-open studios.
2. **locked** — User finishes selecting cards and clicks submit (`brief.submit`). Pending IPC is
   cleared; the blocked `SkillAppPresent` tool returns the VisualBrief (or a continuation prompt carries `[Host VisualBrief …]`).
3. **dismissed** — Host clears `skillAppStore` focus (unmounts iframe) and
   **removes** the `skillApp` conversation tab. Project pins remain for a
   later manual open.

`surface.yaml` `lifecycle: persistent` means **state JSON** survives across
opens; it does **not** keep the UI mounted after handoff.

Do **not** re-present when a `[Host VisualBrief …]` is already in the message,
or via `SkillAppPush` (push must not call `register_present`). Auto-open of the
iframe only reacts to a **new** `wait_brief` `present_id` from `SkillAppPresent`.

### Security

- Serve only files under `ui/` via `/api/skill-apps/{skill_id}/…`.
- Default `network: false`; `host_api` allowlist.
- `vet` scans `ui/` for remote scripts / `eval` / oversized bundles.
- Do **not** use CEF Browser as the Skill App container (singleton + different
  semantics).
- Do **not** reuse deliverable iframe `allow-same-origin`.

### Out of scope (this ADR)

- Skill App marketplace / remote hot-update.
- Shipping skill JS inside `dashboard-ui` main bundle.
- Full slide drag-edit canvas (v1 is select style card(s) → user submits → model continues).
- `AskUserQuestion.preview` (separate Claude-parity track).

## Consequences

- `SkillManifest` / catalog expose `has_ui` + surface metadata.
- Workbench tab union becomes built-in ∪ bound `skill:<id>`.
- Scaffold / skill-creator emit `ui/` stubs; docs under Agent skills.
- Reference implementation: `anycode-ppt` PPT studio Skill App.
