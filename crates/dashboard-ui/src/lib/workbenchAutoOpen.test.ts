import { describe, expect, it } from "vitest";
import type { TranscriptBlock } from "@/api/types";
import {
  BrowserAutoOpenTracker,
  NavigateMirrorTracker,
  PlanAutoOpenTracker,
} from "@/lib/workbenchAutoOpen";

function browserToolCall(id: string, body = ""): TranscriptBlock {
  return {
    id,
    block_type: "tool_call",
    title: "BrowserNavigate",
    body,
    meta: { name: "BrowserNavigate" },
  } as TranscriptBlock;
}

function mcpNavigate(id: string, url: string): TranscriptBlock {
  return {
    id,
    block_type: "tool_call",
    title: "browser_navigate",
    body: JSON.stringify({ url }),
    meta: { name: "mcp__browser__browser_navigate" },
  } as TranscriptBlock;
}

function textBlock(id: string): TranscriptBlock {
  return { id, block_type: "text", body: "hello" } as TranscriptBlock;
}

describe("BrowserAutoOpenTracker", () => {
  it("opens for each new live Browser tool call (openTab is idempotent)", () => {
    const tracker = new BrowserAutoOpenTracker();
    // Stream is live, no browser blocks yet.
    expect(tracker.ingest([textBlock("t1")], true)).toBe(false);
    // First live Browser call → open.
    expect(tracker.ingest([textBlock("t1"), browserToolCall("b1")], true)).toBe(true);
    // Same call re-ingested (dedupe) → no re-open.
    expect(tracker.ingest([textBlock("t1"), browserToolCall("b1")], true)).toBe(false);
    // A *new* live Browser call also opens (openTab is a no-op if already open).
    expect(tracker.ingest([browserToolCall("b1"), browserToolCall("b2")], true)).toBe(true);
  });

  it("never opens for history replay (stream not live)", () => {
    const tracker = new BrowserAutoOpenTracker();
    expect(tracker.ingest([browserToolCall("old1")], false)).toBe(false);
    // Later stream goes live; the old call stays indexed…
    expect(tracker.ingest([browserToolCall("old1")], true)).toBe(false);
    // …and a genuinely new live call opens the panel.
    expect(
      tracker.ingest([browserToolCall("old1"), browserToolCall("new1")], true),
    ).toBe(true);
  });

  it("does not re-open for calls already present at hydration (mid-stream re-entry)", () => {
    // 用户在流仍 live 时关掉浏览器面板、再点回会话:hydration 时已在
    // blocks 里的 Browser 调用必须静默索引,不得重新弹面板;只有
    // hydration 之后到达的新调用才打开(CEF attach 契约仍成立)。
    const tracker = new BrowserAutoOpenTracker();
    expect(tracker.ingest([browserToolCall("b1")], true)).toBe(false);
    expect(tracker.ingest([browserToolCall("b1")], true)).toBe(false);
    expect(
      tracker.ingest([browserToolCall("b1"), browserToolCall("b2")], true),
    ).toBe(true);
  });

  it("indexes history pagination silently after hydration", () => {
    const tracker = new BrowserAutoOpenTracker();
    expect(tracker.ingest([textBlock("t1")], false)).toBe(false);
    // 翻页补进来的更早历史(含 Browser 调用)不得触发打开。
    expect(tracker.ingest([textBlock("t1"), browserToolCall("old1")], false)).toBe(false);
  });

  it("reset re-arms hydration for a new session", () => {
    const tracker = new BrowserAutoOpenTracker();
    expect(tracker.ingest([browserToolCall("b1")], true)).toBe(false);
    expect(tracker.ingest([browserToolCall("b1"), browserToolCall("b2")], true)).toBe(true);
    tracker.reset();
    expect(tracker.ingest([browserToolCall("b1")], false)).toBe(false);
  });
});

describe("NavigateMirrorTracker", () => {
  it("hydrates silently, then returns each new navigate URL exactly once", () => {
    const tracker = new NavigateMirrorTracker();
    // 首次 ingest(进入会话/重置后)只索引,返回 null——不得为进入前
    // 已存在的 URL 重新导航/重开面板。
    expect(tracker.ingest([mcpNavigate("m1", "https://example.com")])).toBeNull();
    expect(tracker.ingest([mcpNavigate("m1", "https://example.com")])).toBeNull();
    // hydration 之后出现的新 URL 正常镜像,且只镜像一次。
    const next = [mcpNavigate("m1", "https://example.com"), mcpNavigate("m2", "https://a.dev")];
    expect(tracker.ingest(next)).toBe("https://a.dev");
    expect(tracker.ingest(next)).toBeNull();
  });

  it("ignores about:blank and non-navigate blocks", () => {
    const tracker = new NavigateMirrorTracker();
    expect(tracker.ingest([textBlock("t1")])).toBeNull();
    expect(tracker.ingest([mcpNavigate("m1", "about:blank")])).toBeNull();
    expect(tracker.ingest([textBlock("t2")])).toBeNull();
  });

  it("reset re-arms hydration for a new session", () => {
    const tracker = new NavigateMirrorTracker();
    const blocks = [mcpNavigate("m1", "https://example.com")];
    expect(tracker.ingest(blocks)).toBeNull();
    tracker.reset();
    // 重置后再次静默索引;同一 URL 在新会话里也不应立即镜像。
    expect(tracker.ingest(blocks)).toBeNull();
  });
});

describe("PlanAutoOpenTracker", () => {
  it("hydrates silently, opens on the next revision", () => {
    const tracker = new PlanAutoOpenTracker();
    expect(tracker.ingest("2026-08-05T10:00:00Z", 2)).toBe(false);
    expect(tracker.ingest("2026-08-05T10:00:00Z", 2)).toBe(false);
    expect(tracker.ingest("2026-08-05T10:05:00Z", 2)).toBe(true);
    expect(tracker.ingest("2026-08-05T10:05:00Z", 2)).toBe(false);
  });

  it("stays disarmed while the tree is empty", () => {
    const tracker = new PlanAutoOpenTracker();
    expect(tracker.ingest(null, 0)).toBe(false);
    expect(tracker.ingest("2026-08-05T10:00:00Z", 0)).toBe(false);
    // First non-empty revision hydrates silently.
    expect(tracker.ingest("2026-08-05T10:01:00Z", 1)).toBe(false);
    expect(tracker.ingest("2026-08-05T10:02:00Z", 1)).toBe(true);
  });

  it("empty tree after a revision re-arms hydration", () => {
    const tracker = new PlanAutoOpenTracker();
    tracker.ingest("k1", 1);
    tracker.ingest(null, 0);
    expect(tracker.ingest("k1", 1)).toBe(false);
  });
});
