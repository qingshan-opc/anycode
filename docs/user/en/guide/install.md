---
title: Install
description: Install the anyCode desktop app or headless daemon from GitHub Releases.
---

# Install

> **Enterprise-grade, free & open-source Agent** — **self-host** on your machine or private network. BYOK models; data stays local by default.

![anyCode workbench — self-hosted enterprise Agent](/docs/assets/screenshots/home.png)
*Digital Workbench on your hardware — projects, sessions, deliverables, and approvals*

## Recommended (macOS)

1. Open [GitHub Releases](https://github.com/qingjiuzys/anycode/releases)
2. Download **`anyCode_<version>_aarch64.dmg`** (Apple Silicon) or the Intel build
3. Drag **anyCode** into Applications
4. Launch **anyCode** — the embedded Workbench starts automatically

No separate CLI install needed for daily use.

## Linux / Windows

| Platform | Method |
|----------|--------|
| **Linux desktop** | `.deb` / `.AppImage` from Releases (when published) |
| **Linux server** | Install **`anycode-daemon`**, open Workbench in browser |
| **Windows** | `.msi` / `.exe` from Releases (when published) |

One-line installer (Linux):

```bash
curl -fsSL --proto '=https' --tlsv1.2 \
  "https://raw.githubusercontent.com/qingjiuzys/anycode/main/scripts/install.sh" | \
  bash -s -- --repo qingjiuzys/anycode
```

## After install

1. Launch **anyCode** from Applications (or Launchpad)
2. Complete the **setup wizard** (`/setup`) if prompted
3. Send a test chat message

> **Developers:** `http://127.0.0.1:43180` is for `anycode-dashboard-serve` browser E2E only. The shipping product is **anyCode.app**; the embedded API uses an ephemeral loopback port, not a fixed 43180.

## Build from source (developers)

```bash
git clone https://github.com/qingjiuzys/anycode.git
cd anycode
./scripts/sync-desktop-dev.sh --rust
# Shipping DMG: ./scripts/build-desktop-local.sh
```

## Next

- [Quick start](./getting-started)
- [Open the Workbench](./dashboard)
- [Common issues](./troubleshooting)

简体中文: [安装](/docs/zh/guide/install).
