import { describe, expect, it } from "vitest";
import {
  briefThinkingSummary,
  truncateThinkingPreview,
} from "@/lib/agentActivitySummary";

describe("briefThinkingSummary", () => {
  it("keeps short text as-is", () => {
    expect(briefThinkingSummary("先读一下现状")).toBe("先读一下现状");
  });

  it("prefers the last sentence (current beat) when the body is long", () => {
    const long =
      "让我先理解用户当前的情况。用户说还不够好。接下来改状态机和瀑布渲染。";
    expect(briefThinkingSummary(long, 96)).toBe("接下来改状态机和瀑布渲染。");
  });

  it("hard-truncates the trailing window when there is no sentence boundary", () => {
    const long = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".repeat(8);
    const out = briefThinkingSummary(long, 40);
    expect(out.startsWith("…")).toBe(true);
    expect(out.length).toBeLessThanOrEqual(40);
  });
});

describe("truncateThinkingPreview", () => {
  it("defaults to a short one-line preview", () => {
    expect(truncateThinkingPreview("a".repeat(120)).length).toBeLessThanOrEqual(100);
  });
});
