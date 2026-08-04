import type { TranscriptBlock } from "@/api/types";
import { toolStepKey } from "@/lib/transcriptGrouping";

/** Keep in sync with `BROWSER_TOOL_IDS` in crates/tools/src/browser_tools.rs */
export const BROWSER_TOOL_IDS = new Set([
  "BrowserTabs",
  "BrowserNavigate",
  "BrowserSnapshot",
  "BrowserClick",
  "BrowserType",
  "BrowserPressKey",
  "BrowserScroll",
  "BrowserScreenshot",
  "BrowserCdp",
  "BrowserConsole",
]);

/** Playwright MCP browser tools appear as `mcp__browser__browser_navigate`, etc. */
const MCP_BROWSER_RE = /^mcp__browser__/i;
const MCP_NAVIGATE_RE = /browser[_-]?navigate/i;

function toolNameFromBlock(block: TranscriptBlock): string | null {
  const metaName = block.meta?.name;
  if (typeof metaName === "string") {
    const trimmed = metaName.trim();
    if (BROWSER_TOOL_IDS.has(trimmed)) return trimmed;
    if (MCP_BROWSER_RE.test(trimmed)) return trimmed;
  }
  const title = block.title?.trim() ?? "";
  const match = title.match(/^(Browser[A-Za-z]+)\b/);
  if (match && BROWSER_TOOL_IDS.has(match[1]!)) {
    return match[1]!;
  }
  const mcpTitle = title.match(/\b(mcp__browser__[A-Za-z0-9_-]+)\b/i);
  if (mcpTitle) return mcpTitle[1]!;
  return null;
}

export function isMcpBrowserToolName(name: string | null | undefined): boolean {
  if (!name) return false;
  return MCP_BROWSER_RE.test(name.trim());
}

export function isMcpBrowserNavigateName(name: string | null | undefined): boolean {
  if (!name) return false;
  const n = name.trim();
  return MCP_BROWSER_RE.test(n) && MCP_NAVIGATE_RE.test(n);
}

/** True for native Browser* or MCP browser tool_call / tool_result blocks. */
export function isBrowserToolBlock(block: TranscriptBlock): boolean {
  if (block.block_type !== "tool_call" && block.block_type !== "tool_result") {
    return false;
  }
  return toolNameFromBlock(block) !== null;
}

export function browserToolDedupeKey(block: TranscriptBlock): string {
  return toolStepKey(block) ?? block.id;
}

/** Auto-open only for a newly started live Browser tool call during an active stream. */
export function shouldAutoOpenBrowserForBlock(
  block: TranscriptBlock,
  opts: { streamLive: boolean },
): boolean {
  if (!opts.streamLive) return false;
  if (block.block_type !== "tool_call") return false;
  if (!isBrowserToolBlock(block)) return false;
  if (block.meta?.live === false) return false;
  return true;
}

export function collectBrowserToolCallKeys(blocks: TranscriptBlock[]): Set<string> {
  const keys = new Set<string>();
  for (const block of blocks) {
    if (block.block_type !== "tool_call") continue;
    if (!isBrowserToolBlock(block)) continue;
    keys.add(browserToolDedupeKey(block));
  }
  return keys;
}

/**
 * Extract a navigation URL from a browser tool_call / tool_result body or title.
 * Supports native JSON `{"url":"..."}` and MCP navigate payloads.
 */
export function extractBrowserNavigateUrl(block: TranscriptBlock): string | null {
  const name =
    (typeof block.meta?.name === "string" ? block.meta.name : null) ??
    toolNameFromBlock(block);
  const isNavigate =
    name === "BrowserNavigate" || isMcpBrowserNavigateName(name);
  if (!isNavigate && block.block_type === "tool_call") {
    // Still try JSON url on any browser tool_call body.
  } else if (!isNavigate && block.block_type === "tool_result") {
    // Results may contain url in JSON state.
  }

  const sources = [block.body, block.title].filter(Boolean) as string[];
  for (const src of sources) {
    const fromJson = urlFromJsonBlob(src);
    if (fromJson) return fromJson;
    const fromText = src.match(/https?:\/\/[^\s"'<>\\]+/i);
    if (fromText) {
      return fromText[0]!.replace(/[),.;]+$/, "");
    }
  }
  return null;
}

function urlFromJsonBlob(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  // Full JSON object
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      const parsed = JSON.parse(trimmed) as unknown;
      const url = findUrlInJson(parsed);
      if (url) return url;
    } catch {
      /* fall through */
    }
  }
  // Embedded {"url":"..."}
  const m = trimmed.match(/"url"\s*:\s*"((?:https?:|about:)[^"]+)"/i);
  if (m?.[1]) return m[1];
  return null;
}

function findUrlInJson(value: unknown, depth = 0): string | null {
  if (depth > 4 || value == null) return null;
  if (typeof value === "string") {
    if (/^https?:\/\//i.test(value) || value.startsWith("about:")) return value;
    return null;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findUrlInJson(item, depth + 1);
      if (found) return found;
    }
    return null;
  }
  if (typeof value === "object") {
    const obj = value as Record<string, unknown>;
    if (typeof obj.url === "string" && obj.url.trim()) return obj.url.trim();
    for (const v of Object.values(obj)) {
      const found = findUrlInJson(v, depth + 1);
      if (found) return found;
    }
  }
  return null;
}

/**
 * Sync MCP navigate into the workbench CDP panel.
 * Native `BrowserNavigate` already drives the shared session — do not mirror it
 * (avoids duplicate create/navigate races that spawn a second Chromium).
 */
export function shouldMirrorNavigateToWorkbench(block: TranscriptBlock): boolean {
  if (block.block_type !== "tool_call" && block.block_type !== "tool_result") {
    return false;
  }
  const name =
    (typeof block.meta?.name === "string" ? block.meta.name : null) ??
    toolNameFromBlock(block);
  if (!isMcpBrowserNavigateName(name)) return false;
  return Boolean(extractBrowserNavigateUrl(block));
}
