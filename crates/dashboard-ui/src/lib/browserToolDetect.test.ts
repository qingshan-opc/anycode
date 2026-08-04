import { describe, expect, it } from "vitest";
import type { TranscriptBlock } from "@/api/types";
import {
  collectBrowserToolCallKeys,
  extractBrowserNavigateUrl,
  isBrowserToolBlock,
  isMcpBrowserNavigateName,
  shouldAutoOpenBrowserForBlock,
  shouldMirrorNavigateToWorkbench,
} from "./browserToolDetect";

function block(
  partial: Partial<TranscriptBlock> & Pick<TranscriptBlock, "id" | "block_type">,
): TranscriptBlock {
  return {
    at: "2026-01-01T00:00:00Z",
    title: "",
    body: "",
    ...partial,
  };
}

describe("browserToolDetect", () => {
  it("matches Browser* and mcp__browser__ tool calls", () => {
    expect(
      isBrowserToolBlock(
        block({
          id: "1",
          block_type: "tool_call",
          meta: { name: "BrowserNavigate" },
        }),
      ),
    ).toBe(true);
    expect(
      isBrowserToolBlock(
        block({
          id: "mcp",
          block_type: "tool_call",
          meta: { name: "mcp__browser__browser_navigate" },
        }),
      ),
    ).toBe(true);
    expect(
      isBrowserToolBlock(
        block({
          id: "2",
          block_type: "assistant_message",
          body: "open the browser please",
        }),
      ),
    ).toBe(false);
    expect(
      isBrowserToolBlock(
        block({
          id: "3",
          block_type: "tool_call",
          title: "Bash started",
          body: "curl browser.example.com",
        }),
      ),
    ).toBe(false);
  });

  it("detects MCP navigate tool names", () => {
    expect(isMcpBrowserNavigateName("mcp__browser__browser_navigate")).toBe(true);
    expect(isMcpBrowserNavigateName("mcp__browser__browser_click")).toBe(false);
  });

  it("auto-opens only for live browser tool_call during stream", () => {
    const browserCall = block({
      id: "1",
      block_type: "tool_call",
      meta: { name: "BrowserSnapshot", live: true },
    });
    expect(shouldAutoOpenBrowserForBlock(browserCall, { streamLive: true })).toBe(true);
    expect(shouldAutoOpenBrowserForBlock(browserCall, { streamLive: false })).toBe(false);
    expect(
      shouldAutoOpenBrowserForBlock(
        block({
          id: "mcp",
          block_type: "tool_call",
          meta: { name: "mcp__browser__browser_navigate", live: true },
          body: '{"url":"https://www.baidu.com"}',
        }),
        { streamLive: true },
      ),
    ).toBe(true);
    expect(
      shouldAutoOpenBrowserForBlock(
        block({
          id: "2",
          block_type: "tool_result",
          meta: { name: "BrowserSnapshot" },
        }),
        { streamLive: true },
      ),
    ).toBe(false);
  });

  it("extracts navigate URLs; mirrors MCP only (not native BrowserNavigate)", () => {
    const mcpCall = block({
      id: "n1",
      block_type: "tool_call",
      meta: { name: "mcp__browser__browser_navigate" },
      body: '{"url":"https://www.baidu.com"}',
    });
    expect(extractBrowserNavigateUrl(mcpCall)).toBe("https://www.baidu.com");
    expect(shouldMirrorNavigateToWorkbench(mcpCall)).toBe(true);

    const native = block({
      id: "n2",
      block_type: "tool_call",
      meta: { name: "BrowserNavigate" },
      body: 'url: https://example.com/path',
    });
    expect(extractBrowserNavigateUrl(native)).toBe("https://example.com/path");
    expect(shouldMirrorNavigateToWorkbench(native)).toBe(false);
  });

  it("collects dedupe keys for browser tool calls", () => {
    const keys = collectBrowserToolCallKeys([
      block({
        id: "a",
        block_type: "tool_call",
        meta: { name: "BrowserNavigate", tool_key: "1:1" },
      }),
      block({
        id: "b",
        block_type: "tool_call",
        title: "Grep started",
        meta: { name: "Grep" },
      }),
      block({
        id: "c",
        block_type: "tool_call",
        meta: { name: "mcp__browser__browser_navigate", tool_key: "2:1" },
      }),
    ]);
    expect(keys.has("1:1")).toBe(true);
    expect(keys.has("2:1")).toBe(true);
    expect(keys.size).toBe(2);
  });
});
