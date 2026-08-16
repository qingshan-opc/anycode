import {
  toolStepRunning,
  type ToolStep,
  type TurnReplyItem,
} from "@/lib/transcriptGrouping";

/** Whether the tool trace should use the live/streaming layout. */
export function toolTraceStreaming(
  steps: ToolStep[],
  processMessageCount: number,
  segmentActive: boolean,
): boolean {
  // Session/segment already settled → never spin, even with unpaired tool_call.
  if (!segmentActive) {
    return false;
  }
  if (steps.some(toolStepRunning)) {
    return true;
  }
  // Pre-tool wait, or post-tool reasoning while the tip cluster is still live.
  if (processMessageCount > 0) {
    return true;
  }
  return steps.length === 0;
}

/**
 * Show the thinking fold while the tip cluster is live and we have reasoning
 * snippets — including *after* tools finish (Codex/Cursor mid-turn).
 */
export function toolTraceShowThinkingHeader(
  _steps: ToolStep[],
  processMessageCount: number,
  segmentActive: boolean,
): boolean {
  return segmentActive && processMessageCount > 0;
}

/** True when a later assistant reply or tool round has already started. */
export function toolClusterSegmentSettled(
  replyItems: TurnReplyItem[],
  clusterIndex: number,
): boolean {
  for (let i = clusterIndex + 1; i < replyItems.length; i++) {
    const row = replyItems[i]!;
    if (row.kind === "tool_cluster") {
      return true;
    }
    if (row.kind === "block") {
      const block = row.block;
      if (block.block_type === "progress_update") {
        return true;
      }
      // Later waterfall prose (thinking / mid-turn narration) settles the prior
      // tool pill — Cursor keeps the completed round as a compact record.
      if (
        block.block_type === "system_notice" &&
        (block.meta?.source === "intermediate_assistant" ||
          block.meta?.source === "thinking_delta") &&
        (block.body?.trim()?.length ?? 0) > 0
      ) {
        return true;
      }
      if (block.block_type === "assistant_message") {
        const body = block.body?.trim() ?? "";
        if (body.length > 0) {
          return true;
        }
      }
    }
  }
  return false;
}

/**
 * Tip cluster on a running turn stays live until a later reply settles it.
 * Tools finishing mid-turn must not collapse thinking into a one-line summary
 * (that hid long context in the composer pill only).
 */
export function toolClusterSegmentActive(
  _steps: ToolStep[],
  sessionRunning: boolean,
  isLastClusterOnLastTurn: boolean,
  settled: boolean,
): boolean {
  if (!sessionRunning || !isLastClusterOnLastTurn || settled) {
    return false;
  }
  // Keep the tip live for running tools, pre-tool wait, and post-tool thinking.
  return true;
}

function isNarrationLikeBlock(block: {
  block_type: string;
  meta?: Record<string, unknown> | null;
  body?: string;
}): boolean {
  if (block.block_type === "progress_update") return true;
  if (
    block.block_type === "system_notice" &&
    (block.meta?.source === "intermediate_assistant" ||
      block.meta?.source === "thinking_delta" ||
      block.meta?.source === "llm_start")
  ) {
    return true;
  }
  if (
    block.block_type === "assistant_message" &&
    (block.meta?.narration === true || block.meta?.message_role === "status")
  ) {
    return true;
  }
  return false;
}

/**
 * Index of the single timeline segment that should stay expanded (accordion).
 * While the turn is running: always the tip (last) segment.
 * When settled: -1 (tools stay one-line; final assistant opened separately).
 */
export function resolveActiveReplySegment(
  replyItems: TurnReplyItem[],
  opts: { isLast: boolean; isRunning: boolean },
): number {
  if (!opts.isLast || !opts.isRunning || replyItems.length === 0) {
    return -1;
  }

  // Codex/Cursor-style: the newest timeline item is always the open one.
  // Prior segments collapse as soon as a newer block/cluster arrives.
  return replyItems.length - 1;
}

/** Last non-narration assistant_message index (final deliverable bubble). */
export function resolveFinalAssistantIndex(replyItems: TurnReplyItem[]): number {
  for (let i = replyItems.length - 1; i >= 0; i--) {
    const item = replyItems[i]!;
    if (item.kind !== "block") continue;
    const block = item.block;
    if (block.block_type !== "assistant_message") continue;
    if (isNarrationLikeBlock(block)) continue;
    if ((block.body?.trim()?.length ?? 0) > 0 || block.meta?.live === true) {
      return i;
    }
  }
  return -1;
}
