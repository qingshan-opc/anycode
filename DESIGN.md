# anyCode Design Contract

Product positioning: **local-first, cloud optional** — an extensible local AI platform for running Agents, connecting local or BYOK models, and adding cloud inference only when needed.

## Brand

- **Mark**: `A` monogram — a single continuous, forward-leaning letterform inside a rounded square. It represents Agent, adaptable local models, and anyCode. It must remain legible at 16px.
- **Wordmark**: `anyCode` (camelCase C) in UI; `anycode` in URLs and package ids.
- **Canonical assets**: `brand/anycode-mark.svg` and `brand/anycode-wordmark.svg`. React components import the canonical mark; generated PNG, favicon, ICO, and ICNS files are outputs and must not be hand-edited.
- **Generation**: `python3 scripts/generate-brand-assets.py` deterministically derives Portal, Workbench, and Desktop assets from the canonical mark.

## Color (dark tool aesthetic)

| Token | Value | Use |
|-------|-------|-----|
| `--accent` | `#6e6bff` (indigo skin) | Primary actions, focus |
| `--surface` | `#0f1117` / `#faf8ff` light | App background |
| `--on-surface` | `#e8eaed` dark / `#131b2e` light | Body text |
| `--outline-variant` | `#3a3f4b` dark / `#c3c6d7` light | Borders |
| `--error` | `#ba1a1a` | Destructive |

Skins: `indigo` (default), `mono`, `coral` — set via `data-skin` on `:root`.

## Typography

- **UI (zh)**: `PingFang SC`, `Microsoft YaHei`, system-ui
- **UI (en/nums)**: `Inter`, system-ui — no blocking remote font loads
- **Code**: `JetBrains Mono`, ui-monospace

## Radius & spacing

- **Single scale**: `--radius-sm: 8px`, `--radius-md: 12px`, `--radius-lg: 16px`
- **Touch target**: minimum 44×44px interactive; language control 36–40px height minimum
- **Top bar**: `--dw-topbar-height: 3rem`

## Motion

- Panel/menu: 120–180ms ease-out
- Message stream: subtle fade-in only
- **No** decorative loops; honor `prefers-reduced-motion: reduce`
- Never `transition: all`

## Accessibility

- Landmarks: `header`, `nav`, `main`
- `:focus-visible` ring on all controls
- WCAG AA contrast for text and controls
- Language switcher: `aria-expanded`, `aria-selected`, Escape to close, keyboard navigation

## Cloud catalog (product)

Only **Cloud Auto** (`auto`), **DeepSeek V4 Flash** (`deepseek-v4-flash`), and **DeepSeek V4 Pro** (`deepseek-v4-pro`) in UI, API, and DB seeds. Agnes is retired.

## Platform capability contract

- **Local Agent runtime** is the product core: projects, tools, skills, approvals, and automations execute locally.
- **Extensible local models** are a platform capability, not a promise tied to one model family. Product copy should describe managed and user-configured local models without presenting MiniCPM or any benchmark label as the primary identity.
- **Optional cloud inference** is limited to Cloud Auto and DeepSeek V4 Flash/Pro in public product surfaces.
- **Native media on macOS** includes Apple Speech input, Apple Vision OCR, local TTS, and Keychain-backed credential storage.
- Avoid claims such as “first”, “SOTA”, “best”, or “fully approved” unless a dated, reviewable evidence record exists.

## Surfaces

- **Desktop (Mac / Windows)**: local Agent only — local projects and local sessions. The app still registers as `home_device` and executes remote prompts. It must not show a cloud remote-conversation list or a device switcher.
- **Phone (`/m`)**: WeChat hop login through the same lingxi Official Account as the portal (same OpenID / `lingxi_user_id`). Lists this account’s computers and commands those conversations. Touch targets stay at least 44×44.
