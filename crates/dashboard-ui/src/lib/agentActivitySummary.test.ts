import { describe, expect, it } from "vitest";
import {
  briefThinkingSummary,
  truncateThinkingPreview,
} from "@/lib/agentActivitySummary";

describe("briefThinkingSummary", () => {
  it("keeps short text as-is", () => {
    expect(briefThinkingSummary("先读一下现状")).toBe("先读一下现状");
  });

  it("prefers the first sentence when the body is long", () => {
    const long =
      "先定位工具簇收起的问题。后面再改状态机和瀑布渲染，确保和 Cursor 一致。";
    expect(briefThinkingSummary(long, 96)).toBe("先定位工具簇收起的问题。");
  });

  it("hard-truncates when there is no short sentence boundary", () => {
    const long = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".repeat(8);
    const out = briefThinkingSummary(long, 40);
    expect(out.endsWith("…")).toBe(true);
    expect(out.length).toBeLessThanOrEqual(40);
  });
});

describe("truncateThinkingPreview", () => {
  it("defaults to a short one-line preview", () => {
    expect(truncateThinkingPreview("a".repeat(120)).length).toBeLessThanOrEqual(100);
  });
});
