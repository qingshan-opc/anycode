import type { TranscriptBlock } from "@/api/types";

export interface ConversationTurn {
  id: string;
  user: TranscriptBlock;
  replies: TranscriptBlock[];
}

const REPLY_BLOCK_TYPES = new Set([
  "assistant_message",
  "session_error",
  "tool_call",
  "tool_result",
  "system_notice",
  "deliverable",
  "progress_update",
]);

function isReplyBlock(blockType: string): boolean {
  return REPLY_BLOCK_TYPES.has(blockType);
}

/** Split flat transcript blocks into user turns + reply chains (used by ConversationTranscript). */
export function blocksToTurns(blocks: TranscriptBlock[]): ConversationTurn[] {
  const turns: ConversationTurn[] = [];
  let current: ConversationTurn | null = null;

  for (const block of blocks) {
    if (block.block_type === "user_message") {
      if (current) turns.push(current);
      current = { id: block.id, user: block, replies: [] };
      continue;
    }
    if (!current) continue;
    if (isReplyBlock(block.block_type)) {
      current.replies.push(block);
    }
  }
  if (current) turns.push(current);
  return turns;
}
