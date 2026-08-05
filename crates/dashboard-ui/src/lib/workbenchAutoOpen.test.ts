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

  it("opens for a browser call already present when the stream starts live", () => {
    // Hydration happens while live (e.g. user opens an already-running session):
    // any unseen Browser call in the current live blocks must open the panel so
    // the embedded CEF creates a page before Agent tools attach.
    const tracker = new BrowserAutoOpenTracker();
    expect(tracker.ingest([browserToolCall("b1")], true)).toBe(true);
  });

  it("reset re-arms hydration for a new session", () => {
    const tracker = new BrowserAutoOpenTracker();
    expect(tracker.ingest([browserToolCall("b1")], true)).toBe(true);
    tracker.reset();
    expect(tracker.ingest([browserToolCall("b1")], false)).toBe(false);
  });
});

describe("NavigateMirrorTracker", () => {
  it("returns each navigate URL exactly once", () => {
    const tracker = new NavigateMirrorTracker();
    const blocks = [mcpNavigate("m1", "https://example.com")];
    expect(tracker.ingest(blocks)).toBe("https://example.com");
    expect(tracker.ingest(blocks)).toBeNull();
  });

  it("ignores about:blank and non-navigate blocks", () => {
    const tracker = new NavigateMirrorTracker();
    expect(tracker.ingest([mcpNavigate("m1", "about:blank")])).toBeNull();
    expect(tracker.ingest([textBlock("t1")])).toBeNull();
  });

  it("reset clears dedupe for a new session", () => {
    const tracker = new NavigateMirrorTracker();
    const blocks = [mcpNavigate("m1", "https://example.com")];
    expect(tracker.ingest(blocks)).toBe("https://example.com");
    tracker.reset();
    expect(tracker.ingest(blocks)).toBe("https://example.com");
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
