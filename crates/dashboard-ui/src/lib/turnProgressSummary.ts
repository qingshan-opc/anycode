import type { TranscriptBlock } from "@/api/types";
import { sanitizeAssistantDisplay } from "@/lib/assistantText";
import { dedupeNarrationWithProgress } from "@/lib/phaseGrouping";
import { progressSummary } from "@/lib/progressMeta";
import { groupTurnReplies, mergeFinalAssistantBlocks } from "@/lib/transcriptGrouping";
import { groupTurnForWorkLog, latestWorkSummary } from "@/lib/workLogGrouping";

const REPLY_BLOCK_TYPES = new Set([
  "assistant_message",
  "session_error",
  "tool_call",
  "tool_result",
  "system_notice",
  "deliverable",
  "progress_update",
]);

/** Replies after the last user_message — same slice TurnRecapHeader summarizes. */
export function lastTurnReplies(blocks: TranscriptBlock[]): TranscriptBlock[] {
  let replies: TranscriptBlock[] = [];
  let started = false;
  for (const block of blocks) {
    if (block.block_type === "user_message") {
      replies = [];
      started = true;
      continue;
    }
    if (!started) continue;
    if (REPLY_BLOCK_TYPES.has(block.block_type)) {
      replies.push(block);
    }
  }
  return replies;
}

/** Auto-updating progress / narration line shown next to「提交并推送」. */
export function computeLatestTurnProgress(
  blocks: TranscriptBlock[],
  locale: string,
): string | null {
  if (blocks.length === 0) return null;
  const replies = lastTurnReplies(blocks);
  if (replies.length === 0) return null;
  const items = dedupeNarrationWithProgress(
    groupTurnReplies(mergeFinalAssistantBlocks(replies)),
  );
  const work = groupTurnForWorkLog(items).work;
  return latestWorkSummary(work, (block) =>
    progressSummary(block, sanitizeAssistantDisplay(block.body, locale)),
  );
}
