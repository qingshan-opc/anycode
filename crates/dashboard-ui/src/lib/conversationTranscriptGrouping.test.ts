import { describe, expect, it } from "vitest";
import type { TranscriptBlock } from "@/api/types";
import { blocksToTurns } from "@/lib/conversationTranscriptGrouping";
import { groupTurnReplies } from "@/lib/transcriptGrouping";

function block(
  id: string,
  blockType: string,
  extra?: Partial<TranscriptBlock>,
): TranscriptBlock {
  return {
    id,
    block_type: blockType,
    at: "2026-01-01T00:00:00Z",
    title: "",
    body: "",
    ...extra,
  };
}

describe("ConversationTranscript grouping pipeline", () => {
  it("splits blocks into turns and clusters tool evidence per turn", () => {
    const blocks = [
      block("u1", "user_message", { body: "fix the bug" }),
      block("a1", "assistant_message", { body: "checking" }),
      block("t1", "tool_call", { meta: { tool_key: "1:1" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1" } }),
      block("f1", "assistant_message", { body: "done" }),
      block("u2", "user_message", { body: "thanks" }),
      block("a2", "assistant_message", { body: "welcome" }),
    ];

    const turns = blocksToTurns(blocks);
    expect(turns).toHaveLength(2);
    expect(turns[0]?.replies).toHaveLength(4);
    expect(turns[1]?.replies).toHaveLength(1);

    const grouped = groupTurnReplies(turns[0]!.replies);
    expect(grouped.map((item) => item.kind)).toEqual([
      "block",
      "tool_cluster",
      "block",
    ]);
  });

  it("ignores lifecycle blocks outside a user turn", () => {
    const blocks = [
      block("life", "turn_start", { title: "turn_start" }),
      block("u1", "user_message", { body: "hi" }),
      block("a1", "assistant_message", { body: "hello" }),
    ];
    const turns = blocksToTurns(blocks);
    expect(turns).toHaveLength(1);
    expect(turns[0]?.replies.map((r) => r.id)).toEqual(["a1"]);
  });
});
