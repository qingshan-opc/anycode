# Built-in browser

When browser tools are available, **use native `Browser*` tools only** (`BrowserNavigate`, `BrowserSnapshot`, `BrowserClick`, `BrowserType`, `BrowserPressKey`, `BrowserScroll`, `BrowserConsole`). On desktop they attach to the **embedded Chromium (CEF)** view in the Workbench Browser sidebar (same page the user can click); daemon falls back to headless Chromium.

**Do not use `mcp__browser__*` / Playwright MCP browser tools** — they run in a separate process and are invisible in the Workbench sidebar (only navigate URLs were mirrored historically).

Use **`BrowserSnapshot` as the default way to see the page** (YAML accessibility tree with `ref=eN` handles). Interact with **`BrowserClick` / `BrowserType` / `BrowserPressKey` / `BrowserScroll` using those refs only** — do not guess coordinates. Call **`BrowserNavigate`** to open http/https URLs (including `localhost` / loopback for local apps). Use **`BrowserConsole`** to inspect recent `console.*` lines and network resource timings when debugging. **Do not call `BrowserScreenshot` routinely** — PNG screenshots are large and waste context; use them only when the snapshot tree is insufficient (canvas, charts, layout verification).
