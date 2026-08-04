import { describe, expect, it } from "vitest";
import type { TranscriptBlock } from "@/api/types";
import {
  computeLatestTurnProgress,
  lastTurnReplies,
} from "@/lib/turnProgressSummary";

function block(
  partial: Partial<TranscriptBlock> & Pick<TranscriptBlock, "id" | "block_type" | "body">,
): TranscriptBlock {
  return {
    at: "2026-08-04T08:00:00Z",
    title: "",
    ...partial,
  };
}

describe("turnProgressSummary", () => {
  it("lastTurnReplies keeps only the latest user turn", () => {
    const blocks = [
      block({ id: "u1", block_type: "user_message", body: "first" }),
      block({ id: "a1", block_type: "assistant_message", body: "old" }),
      block({ id: "u2", block_type: "user_message", body: "second" }),
      block({
        id: "p1",
        block_type: "assistant_message",
        body: "继续深入 workbench 核心实现与文档。",
        meta: { narration: true },
      }),
    ];
    expect(lastTurnReplies(blocks).map((b) => b.id)).toEqual(["p1"]);
  });

  it("computeLatestTurnProgress returns the latest narration line", () => {
    const blocks = [
      block({ id: "u1", block_type: "user_message", body: "go" }),
      block({
        id: "p1",
        block_type: "assistant_message",
        body: "先看目录。",
        meta: { narration: true },
      }),
      block({
        id: "p2",
        block_type: "assistant_message",
        body: "继续深入 workbench 核心实现与文档。",
        meta: { narration: true },
      }),
    ];
    expect(computeLatestTurnProgress(blocks, "zh")).toBe(
      "继续深入 workbench 核心实现与文档。",
    );
  });
});
