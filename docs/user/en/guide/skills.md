---
title: Agent skills
description: SKILL.md layout, ~/.anycode/skills discovery, config skills.*, and the Skill tool.
summary: How anyCode discovers skills, injects them into the system prompt, and runs optional run scripts.
read_when:
  - You want folder layout and config for Agent Skills–style extensions.
---

# Agent skills

anyCode aligns with common **Agent Skills** conventions: each skill is a directory with a **`SKILL.md`** file whose YAML frontmatter includes **`name`** and **`description`**. An optional executable **`run`** in that directory is invoked by the **`Skill`** tool (same broad risk class as **Bash** — approvals / sensitive-tool policy apply).

## Layout

- **User-wide default root:** `~/.anycode/skills/<skill_id>/`
- **Project overrides (no startup scan):** `<cwd>/skills/<skill_id>/` or `<cwd>/.anycode/skills/<skill_id>/` — resolved when the **Skill** tool runs if the id is not already in the catalog.
- **`skill_id`** must match the directory name and the frontmatter **`name`** (ASCII letters, digits, `.`, `_`, `-` only). Mismatches are skipped with a log warning.

Minimal **`SKILL.md`**:

```markdown
---
name: my-skill
description: One line for the model and for the Workbench skills list.
permissions:
  network: false
---

# my-skill

Longer documentation for humans (optional).
```

Optional **`run`**: must be a regular file. It is executed with the **task working directory** as **cwd** (not the skill directory). Env vars `ANYCODE_SKILL_DIR` and `ANYCODE_WORKING_DIR` are set. By default a **minimal env** is used (`skills.minimal_env`, default `true`).

If frontmatter sets **`permissions.network: false`**, the host **refuses to execute `run`** (fail-closed). Load instructions with the Skill tool and no `args` instead.

## Config (`~/.anycode/config.json`)

Under **`skills`**:

| Field | Meaning |
|-------|---------|
| **`enabled`** | When `true`, scan **`skills.extra_dirs`** then **`~/.anycode/skills`** at startup; build catalog and inject **## Available skills** into the default system stack (skipped when **`system_prompt_override`** is set). |
| **`extra_dirs`** | Extra scan roots (lower precedence than **`~/.anycode/skills`**; later roots override same id). |
| **`allowlist`** | If set, only these ids appear in the catalog and prompt. |
| **`run_timeout_ms`** | Subprocess timeout for **`run`** (minimum enforced in code). |
| **`minimal_env`** | When `true` (default), only a small env whitelist (**PATH**, **HOME**, **USER**, etc.) is passed to **`run`**. |
| **`expose_on_explore_plan`** | When `true` **and** **`enabled`**, **explore** / **plan** agents also get the **Skill** tool (default `false` to limit code execution surface). |

## Workbench

Manage and enable skills in **Settings → Skills**. There is no terminal `anycode skills` CLI (the CLI product surface was removed).

## Skill Apps (visual workbench)

A skill may ship a sandboxed mini-app under `ui/` (see [ADR 020](https://github.com/qingjiuzys/anycode/blob/main/docs/adr/020-skill-apps.md)):

```text
my-skill/
  SKILL.md
  ui/
    surface.yaml
    index.html
```

The Workbench mounts the app in the **dock**, **conversation** tab, or as a **project pin**. Agent tools:

| Tool | Role |
|------|------|
| **SkillAppPresent** | Primary path: open the PPT / short-video studio with `wait="brief"` so the user picks style/templates, then follow the returned VisualBrief. Do not call again when a VisualBrief is already in the user message. |
| **SkillAppPush** | Non-blocking preview/progress into the open app |
| **SkillAppRead** | Read current brief/state for the project |

Host bridge (postMessage JSON-RPC): `anycode.state.*`, `anycode.brief.submit`, `anycode.agent.prompt`. Skills must not call dashboard REST from the iframe. Reference: `skills-starter/skill-app-hello`, `skills-starter/anycode-ppt/ui`, `skills-starter/anycode-video/ui`.

The bundled **anycode-ppt** studio: pick a visual **skin** (Open Design families or color palettes — FDE, Apple Keynote, …), then click **交给 Agent**. The model opens the studio via `SkillAppPresent` and resumes after the brief. Page count and main visuals are inferred by the model — do not pad to 12 template pages.

The bundled **anycode-video** studio: pick aspect (9:16 / 16:9 / 1:1) and one or more html-video motion templates, then click **交给 Agent**. Agent fills template inputs only, previews HTML in the workbench, and `run` exports MP4 via Playwright + ffmpeg (not cloud `GenerateVideo`).

## Model visibility

When skills are enabled and the default system prompt stack is used, the prompt includes an **Available skills** section listing ids and descriptions. Execution still goes through the **Skill** tool with **`{"name": "<id>", "args": [...]}`**.

## Related

- Maintainer architecture: `docs/architecture.md`
- Skill starter packs: `skills-starter/`
- ADR: `docs/adr/020-skill-apps.md`
