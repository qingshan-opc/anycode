import type { TranscriptBlock } from "@/api/types";

export type AgentPhaseKind = "intent" | "execute" | "discovery" | "deliver";

export function isProgressBlock(block: TranscriptBlock): boolean {
  return (
    block.block_type === "progress_update" ||
    (block.block_type === "assistant_message" &&
      (block.meta?.narration === true || block.meta?.message_role === "status"))
  );
}

/** One-line agent status updates — show inline, not as a fold control (thinking/tools stay expandable). */
export function isStaticProgressStatusLine(block: TranscriptBlock): boolean {
  if (block.block_type === "progress_update") return true;
  if (
    block.block_type === "system_notice" &&
    block.meta?.source === "intermediate_assistant"
  ) {
    return true;
  }
  return (
    block.block_type === "assistant_message" &&
    (block.meta?.narration === true ||
      block.meta?.message_role === "status" ||
      block.meta?.live === true)
  );
}

/** Mid-turn assistant text — inline status, not a final reply bubble. */
export function shouldRenderAssistantAsStatusLine(
  block: TranscriptBlock,
  opts: { itemIndex: number; finalAssistantIndex: number },
): boolean {
  if (block.block_type !== "assistant_message") return false;
  if (isStaticProgressStatusLine(block)) return true;
  if (opts.finalAssistantIndex >= 0 && opts.itemIndex !== opts.finalAssistantIndex) {
    return true;
  }
  return false;
}

export function progressPhase(block: TranscriptBlock): AgentPhaseKind {
  const raw = block.meta?.phase;
  if (typeof raw === "string") {
    if (raw === "intent" || raw === "execute" || raw === "discovery" || raw === "deliver") {
      return raw;
    }
    // Compile / gate / skill markers are preflight (intent).
    if (raw === "gate" || raw === "skill" || raw === "compile") {
      return "intent";
    }
  }
  if (block.meta?.work_stage === "compile") {
    return "intent";
  }
  if (block.meta?.narration === true || block.meta?.message_role === "status") {
    return "execute";
  }
  return "execute";
}

/**
 * Delivery preflight markers (`[delivery_preflight] …`) are internal diagnostics —
 * never rendered as visible text in the conversation. Kept as a no-op so callers
 * (e.g. ToolTraceCluster) fall back to their own snippet text.
 */
export function formatDeliveryPreflight(_summary: string): string | null {
  return null;
}

export function progressSummary(block: TranscriptBlock, localeBody?: string): string {
  const metaSummary = block.meta?.summary;
  const raw =
    typeof metaSummary === "string" && metaSummary.trim()
      ? metaSummary.trim()
      : (localeBody ?? block.body ?? "").trim();
  if (raw.includes("[delivery_preflight]")) return "";
  return raw;
}

export function progressNext(block: TranscriptBlock): string | null {
  const raw = block.meta?.next;
  return typeof raw === "string" && raw.trim() ? raw.trim() : null;
}

export function progressDiscovery(block: TranscriptBlock): string | null {
  const raw = block.meta?.discovery;
  return typeof raw === "string" && raw.trim() ? raw.trim() : null;
}

export function progressWorkStage(block: TranscriptBlock): string | null {
  const raw = block.meta?.work_stage;
  return typeof raw === "string" && raw.trim() ? raw.trim() : null;
}

export function progressEvidenceRefs(block: TranscriptBlock): string[] {
  const raw = block.meta?.evidence_refs;
  if (!Array.isArray(raw)) return [];
  return raw.filter((v): v is string => typeof v === "string");
}

export function phaseTitleKey(phase: AgentPhaseKind): string {
  switch (phase) {
    case "intent":
      return "conversations.progressPhaseIntent";
    case "execute":
      return "conversations.progressPhaseExecute";
    case "discovery":
      return "conversations.progressPhaseDiscovery";
    case "deliver":
      return "conversations.progressPhaseDeliver";
  }
}

export function workStageLabelKey(stage: string): string | null {
  switch (stage) {
    case "inspect":
      return "conversations.progressWorkInspect";
    case "analyze":
      return "conversations.progressWorkAnalyze";
    case "implement":
      return "conversations.progressWorkImplement";
    case "verify":
      return "conversations.progressWorkVerify";
    case "compile":
      return "conversations.progressWorkCompile";
    default:
      return null;
  }
}
