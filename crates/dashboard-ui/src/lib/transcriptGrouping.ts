import type { TranscriptBlock } from "@/api/types";

export type ToolStep = {
  key: string;
  call?: TranscriptBlock;
  result?: TranscriptBlock;
};

export type SubagentGroupItem = {
  kind: "subagent_group";
  id: string;
  taskId: string;
  agentType: string;
  /** null while the nested task is still running. */
  status: string | null;
  /** Inner timeline, grouped with the same rules as a flat segment. */
  items: TurnReplyItem[];
  toolCount: number;
};

export type TurnReplyItem =
  | { kind: "block"; block: TranscriptBlock }
  | {
      kind: "tool_cluster";
      id: string;
      steps: ToolStep[];
      processMessageCount: number;
      /** Collapsed intermediate assistant snippets (thinking). */
      processSnippets: string[];
    }
  | SubagentGroupItem;

/** Subagent scope tag written by the dashboard bridge (`meta.subagent`). */
export function subagentTaskIdOf(block: TranscriptBlock): string | null {
  const sa = block.meta?.subagent as
    | { task_id?: unknown }
    | null
    | undefined;
  const id = sa?.task_id;
  return typeof id === "string" && id.length > 0 ? id : null;
}

function isIntermediateAssistantNotice(block: TranscriptBlock): boolean {
  return (
    block.block_type === "system_notice" &&
    (block.meta?.source === "intermediate_assistant" ||
      block.meta?.source === "llm_start" ||
      block.meta?.source === "thinking_delta")
  );
}

function isNarrationAssistant(block: TranscriptBlock): boolean {
  return (
    block.block_type === "assistant_message" &&
    (block.meta?.narration === true || block.meta?.message_role === "status")
  );
}

function isProgressAssistant(block: TranscriptBlock): boolean {
  return block.block_type === "progress_update" || isNarrationAssistant(block);
}

function isToolBlock(block: TranscriptBlock): boolean {
  return block.block_type === "tool_call" || block.block_type === "tool_result";
}

function buildToolSteps(tools: TranscriptBlock[]): ToolStep[] {
  const byKey = new Map<string, ToolStep>();
  const order: string[] = [];

  for (const tool of tools) {
    const key = toolStepKey(tool) ?? tool.id;
    if (!byKey.has(key)) {
      order.push(key);
      byKey.set(key, { key });
    }
    const slot = byKey.get(key)!;
    if (tool.block_type === "tool_result") {
      slot.result = tool;
    } else {
      slot.call = tool;
    }
  }

  return order.map((key) => byKey.get(key)!);
}

function makeToolCluster(
  tools: TranscriptBlock[],
  processMessageCount: number,
  processSnippets: string[],
): TurnReplyItem {
  return {
    kind: "tool_cluster",
    id: `tools:${tools[0]?.id ?? `process-${processMessageCount}`}`,
    steps: buildToolSteps(tools),
    processMessageCount,
    processSnippets,
  };
}

function pushThinkingSnippet(processSnippets: string[], snippet: string): boolean {
  const trimmed = snippet.trim();
  if (!trimmed) return false;
  const last = processSnippets[processSnippets.length - 1];
  if (last === trimmed) return false;
  processSnippets.push(trimmed);
  return true;
}

/**
 * Merge multiple assistant_message blocks into one final bubble per user turn segment.
 */
export function mergeFinalAssistantBlocks(replies: TranscriptBlock[]): TranscriptBlock[] {
  const out: TranscriptBlock[] = [];
  let buffer: TranscriptBlock[] = [];

  const flush = () => {
    if (buffer.length === 0) return;
    if (buffer.length === 1) {
      out.push(buffer[0]!);
    } else {
      const last = buffer[buffer.length - 1]!;
      const body = buffer
        .map((b) => b.body?.trim() ?? "")
        .filter(Boolean)
        .join("\n\n");
      out.push({
        ...last,
        body,
        meta: {
          ...(last.meta ?? {}),
          live: buffer.some((b) => Boolean(b.meta?.live)),
          merged_assistant: true,
        },
      });
    }
    buffer = [];
  };

  for (const block of replies) {
    if (
      block.block_type === "assistant_message" &&
      !isNarrationAssistant(block)
    ) {
      buffer.push(block);
      continue;
    }
    flush();
    out.push(block);
  }
  flush();
  return out;
}

/**
 * Group tool blocks into per-segment clusters (Cursor/Codex-style interleaving).
 * Agent narration is a user-facing progress update, so it remains a first-class
 * transcript block. Only transport/system notices fold into tool details.
 *
 * Subagent-tagged blocks (`meta.subagent.task_id`) are first split into
 * per-child groups; each group renders as a collapsible nested timeline.
 */
export function groupTurnReplies(replies: TranscriptBlock[]): TurnReplyItem[] {
  const taggedIds = collectTaggedTaskIds(replies);
  return groupScoped(replies, null, new Set(), taggedIds);
}

type SubagentTag = {
  taskId: string;
  agentType?: string;
  parentTaskId: string | null;
};

function subagentTagOf(block: TranscriptBlock): SubagentTag | null {
  const sa = block.meta?.subagent as
    | { task_id?: unknown; agent_type?: unknown; parent_task_id?: unknown }
    | null
    | undefined;
  const id = sa?.task_id;
  if (typeof id !== "string" || id.length === 0) return null;
  return {
    taskId: id,
    agentType:
      typeof sa?.agent_type === "string" && sa.agent_type
        ? sa.agent_type
        : undefined,
    parentTaskId:
      typeof sa?.parent_task_id === "string" && sa.parent_task_id
        ? sa.parent_task_id
        : null,
  };
}

function collectTaggedTaskIds(replies: TranscriptBlock[]): Set<string> {
  const ids = new Set<string>();
  for (const block of replies) {
    const tag = subagentTagOf(block);
    if (tag) ids.add(tag.taskId);
  }
  return ids;
}

/**
 * One grouping pass at a fixed nesting scope:
 * - untagged blocks, and blocks tagged with an ancestor scope's own id, stay
 *   flat at this level (ancestor-tag flattening is what terminates recursion);
 * - blocks whose `parent_task_id` is this scope (or, at root, whose parent is
 *   not a subtask present in this stream) aggregate into per-child groups,
 *   placed at first sight so interleaved parallel children still render as
 *   one card each;
 * - deeper-tagged blocks are skipped here and consumed by the recursion that
 *   builds their ancestor's group.
 */
function groupScoped(
  replies: TranscriptBlock[],
  scopeTaskId: string | null,
  ancestors: Set<string>,
  taggedIds: Set<string>,
): TurnReplyItem[] {
  type Marker =
    | { type: "normal"; blocks: TranscriptBlock[] }
    | { type: "group"; taskId: string };
  const markers: Marker[] = [];
  const groupBlocks = new Map<string, TranscriptBlock[]>();
  let normal: TranscriptBlock[] = [];
  const flushNormal = () => {
    if (normal.length > 0) {
      markers.push({ type: "normal", blocks: normal });
      normal = [];
    }
  };
  for (const block of replies) {
    const tag = subagentTagOf(block);
    if (!tag) {
      // Untagged blocks are the owning task's own activity: they belong to the
      // ancestor scopes, not to a child group's inner timeline.
      if (scopeTaskId === null) {
        normal.push(block);
      }
      continue;
    }
    if (tag.taskId === scopeTaskId) {
      // This scope's own activity renders flat inside its group; the start/done
      // markers already folded into the group card's chrome.
      const source = block.meta?.source;
      if (source !== "subagent_start" && source !== "subagent_done") {
        normal.push(block);
      }
      continue;
    }
    if (ancestors.has(tag.taskId)) {
      continue; // defensive: ancestor-tag cycle, consumed at its own scope
    }
    const parentInStream =
      tag.parentTaskId !== null &&
      taggedIds.has(tag.parentTaskId) &&
      !ancestors.has(tag.parentTaskId);
    const directChild =
      scopeTaskId === null
        ? !parentInStream
        : tag.parentTaskId === scopeTaskId;
    if (!directChild) {
      continue;
    }
    let blocks = groupBlocks.get(tag.taskId);
    if (!blocks) {
      flushNormal();
      blocks = [];
      groupBlocks.set(tag.taskId, blocks);
      markers.push({ type: "group", taskId: tag.taskId });
    }
    blocks.push(block);
  }
  flushNormal();

  const out: TurnReplyItem[] = [];
  for (const marker of markers) {
    if (marker.type === "normal") {
      out.push(...groupFlatTurnReplies(marker.blocks));
    } else {
      const childAncestors = new Set(ancestors);
      childAncestors.add(marker.taskId);
      out.push(
        buildSubagentGroup(
          marker.taskId,
          groupBlocks.get(marker.taskId)!,
          replies,
          childAncestors,
          taggedIds,
        ),
      );
    }
  }
  return out;
}

function buildSubagentGroup(
  taskId: string,
  blocks: TranscriptBlock[],
  replies: TranscriptBlock[],
  ancestors: Set<string>,
  taggedIds: Set<string>,
): SubagentGroupItem {
  let agentType = "subagent";
  let status: string | null = null;
  const inner: TranscriptBlock[] = [];
  for (const block of blocks) {
    const sa = block.meta?.subagent as { agent_type?: unknown } | undefined;
    if (typeof sa?.agent_type === "string" && sa.agent_type) {
      agentType = sa.agent_type;
    }
    const source = block.meta?.source;
    if (source === "subagent_start") {
      continue; // header → group chrome
    }
    if (source === "subagent_done") {
      status =
        typeof block.meta?.status === "string" ? block.meta.status : "completed";
      continue; // done marker → group status
    }
    inner.push(block);
  }
  return {
    kind: "subagent_group",
    id: `subagent-group:${taskId}`,
    taskId,
    agentType,
    status,
    // Recurse at this group's scope: own-tag blocks flatten (ancestors guard),
    // grandchild-tagged blocks form their own nested subagent_group cards.
    items: groupScoped(replies, taskId, ancestors, taggedIds),
    toolCount: countLogicalToolSteps(inner.filter(isToolBlock)),
  };
}

function groupFlatTurnReplies(replies: TranscriptBlock[]): TurnReplyItem[] {
  const out: TurnReplyItem[] = [];
  let toolBuffer: TranscriptBlock[] = [];
  let processCount = 0;
  const processSnippets: string[] = [];

  const flushTools = () => {
    if (toolBuffer.length === 0 && processCount === 0 && processSnippets.length === 0) {
      return;
    }
    out.push(makeToolCluster(toolBuffer, processCount, [...processSnippets]));
    toolBuffer = [];
    processCount = 0;
    processSnippets.length = 0;
  };

  for (let index = 0; index < replies.length; index += 1) {
    const block = replies[index]!;
    if (isToolBlock(block)) {
      toolBuffer.push(block);
      continue;
    }

    if (isIntermediateAssistantNotice(block)) {
      const snippet = block.body?.trim() ?? "";
      const source = block.meta?.source;
      // Cursor waterfall: keep thinking / mid-turn narration as timeline prose
      // that stays above the following tool pill. Only empty transport notices
      // (llm_start) fold into the cluster chrome.
      if (
        (source === "intermediate_assistant" || source === "thinking_delta") &&
        snippet.length > 0
      ) {
        flushTools();
        out.push({ kind: "block", block });
        continue;
      }
      if (snippet) {
        if (pushThinkingSnippet(processSnippets, snippet)) {
          processCount += 1;
        }
      } else {
        processCount += 1;
      }
      continue;
    }

    if (block.block_type === "progress_update") {
      // Heuristic progress_update is not a timeline hero — skip so tool clusters
      // stay contiguous under assistant narration (Cursor/Codex style).
      continue;
    }

    if (isNarrationAssistant(block)) {
      flushTools();
      out.push({ kind: "block", block });
      continue;
    }

    if (block.block_type === "assistant_message") {
      const body = block.body?.trim() ?? "";
      if (body.length === 0 && block.meta?.live !== true) {
        processCount += 1;
        continue;
      }
      // Keep mid-turn assistant narration on the timeline (Claude/Codex-style),
      // even when more tools follow. Do not fold into tool thinking snippets.
      flushTools();
      out.push({ kind: "block", block });
      continue;
    }

    flushTools();
    out.push({ kind: "block", block });
  }

  flushTools();
  return mergeToolClusters(out);
}

function isClusterMergeSeparator(block: TranscriptBlock): boolean {
  if (isProgressAssistant(block)) return false;
  if (isIntermediateAssistantNotice(block)) return false;
  if (block.block_type === "system_notice") return true;
  if (block.block_type === "assistant_message") {
    const body = block.body?.trim() ?? "";
    return body.length === 0 && block.meta?.live !== true;
  }
  return false;
}

function mergeClusterItems(
  left: Extract<TurnReplyItem, { kind: "tool_cluster" }>,
  right: Extract<TurnReplyItem, { kind: "tool_cluster" }>,
  sepSnippets: string[],
  sepCount: number,
): Extract<TurnReplyItem, { kind: "tool_cluster" }> {
  const snippets = [...left.processSnippets];
  for (const snippet of sepSnippets) {
    pushThinkingSnippet(snippets, snippet);
  }
  return {
    kind: "tool_cluster",
    id: left.id,
    steps: [...left.steps, ...right.steps],
    processMessageCount: left.processMessageCount + sepCount + right.processMessageCount,
    processSnippets: [...snippets, ...right.processSnippets],
  };
}

/** Merge tool clusters separated only by narration / status blocks. */
function mergeToolClusters(items: TurnReplyItem[]): TurnReplyItem[] {
  const out: TurnReplyItem[] = [];
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i]!;
    if (item.kind !== "tool_cluster") {
      out.push(item);
      continue;
    }

    let merged: Extract<TurnReplyItem, { kind: "tool_cluster" }> = {
      kind: "tool_cluster",
      id: item.id,
      steps: [...item.steps],
      processMessageCount: item.processMessageCount,
      processSnippets: [...item.processSnippets],
    };

    let j = i + 1;
    while (j < items.length) {
      const sepSnippets: string[] = [];
      let sepCount = 0;
      let k = j;
      while (k < items.length) {
        const mid = items[k]!;
        if (mid.kind === "block" && isClusterMergeSeparator(mid.block)) {
          const snippet = mid.block.body?.trim();
          if (snippet) {
            pushThinkingSnippet(sepSnippets, snippet);
          } else {
            sepCount += 1;
          }
          k += 1;
          continue;
        }
        break;
      }
      const next = items[k];
      if (next?.kind === "tool_cluster" && k > j) {
        merged = mergeClusterItems(merged, next, sepSnippets, sepCount);
        i = k;
        j = k + 1;
        continue;
      }
      break;
    }

    const last = out[out.length - 1];
    if (last?.kind === "tool_cluster") {
      last.steps = [...last.steps, ...merged.steps];
      last.processMessageCount += merged.processMessageCount;
      last.processSnippets = [...last.processSnippets, ...merged.processSnippets];
    } else {
      out.push(merged);
    }
  }
  return out;
}

/** Count logical tool invocations (paired start/end), not raw transcript blocks. */
export function countLogicalToolSteps(tools: TranscriptBlock[]): number {
  const keys = new Set<string>();
  for (const tool of tools) {
    const key = toolStepKey(tool);
    if (key) {
      keys.add(key);
    }
  }
  if (keys.size > 0) {
    return keys.size;
  }
  const calls = tools.filter((t) => t.block_type === "tool_call").length;
  if (calls > 0) {
    return calls;
  }
  return Math.max(1, Math.ceil(tools.length / 2));
}

export function toolStepKey(tool: TranscriptBlock): string | null {
  const meta = tool.meta;
  if (!meta) {
    return null;
  }
  const toolKey = meta.tool_key;
  if (typeof toolKey === "string" && toolKey.trim()) {
    return toolKey.trim();
  }
  const turn = meta.turn;
  const idx = meta.idx;
  if (typeof turn === "string" && typeof idx === "string" && turn && idx) {
    return `${turn}:${idx}`;
  }
  return tool.event_id ?? tool.id;
}

export function toolStepRunning(step: ToolStep): boolean {
  return Boolean(step.call) && !step.result;
}

const TOOL_FAILED_TITLE_RE = /\bfailed\b/i;
const TOOL_FINISHED_TITLE_RE = /\bfinished\b/i;

/** 纯文本错误前缀：与输出正文无关的确定性失败信号。 */
const TEXT_FAILURE_MARKERS = [
  "Command failed",
  "Command timed out",
  "File not found",
  "Not a file",
  "Is a directory",
  "Permission denied",
  "rg failed",
  "Serialization error: missing field",
  "path escapes sandbox",
  "skill exited with code",
];

export function toolResultFailed(title: string, body: string): boolean {
  // 1) 服务端生成的 title（"{name} failed/finished"）是最可靠信号。
  if (TOOL_FINISHED_TITLE_RE.test(title)) return false;
  if (TOOL_FAILED_TITLE_RE.test(title)) return true;

  // 2) 结构化 JSON 优先：exit_code / success / error 字段。
  try {
    const parsed = JSON.parse(body);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const obj = parsed as Record<string, unknown>;
      if (typeof obj.exit_code === "number") return obj.exit_code !== 0;
      if (typeof obj.success === "boolean") return !obj.success;
      if (typeof obj.error === "string" && obj.error) return true;
      // 成功负载字段：FileRead(content) / Glob(filenames) / Grep(matches) / WebSearch(raw)
      if ("content" in obj || "filenames" in obj || "matches" in obj || "raw" in obj) {
        return false;
      }
    }
  } catch {
    // 非 JSON，走纯文本判定。
  }

  // 3) 明确错误前缀，避免把输出正文里的 error/failed 字样误判为失败。
  const trimmed = body.trim();
  if (TEXT_FAILURE_MARKERS.some((m) => trimmed.startsWith(m))) return true;
  if (/^HTTP [45]\d\d\b/.test(trimmed)) return true;
  if (/^Other error:/.test(trimmed)) return true;

  return false;
}

export function toolStepFailed(step: ToolStep): boolean {
  const primary = step.result ?? step.call;
  if (!primary) return false;
  return toolResultFailed(primary.title, primary.body);
}

/** Name of the tool currently running in a turn, if any. */
export function findActiveToolInReplies(replies: TranscriptBlock[]): string | null {
  const steps = buildToolSteps(replies.filter(isToolBlock));
  for (let i = steps.length - 1; i >= 0; i -= 1) {
    const step = steps[i]!;
    if (toolStepRunning(step)) {
      return step.call?.title?.replace(/\s+started$/i, "") ?? step.call?.title ?? null;
    }
  }
  return null;
}

export function findActiveToolInExecutionLog(
  lines: { event_type?: string | null; title?: string | null; raw: string }[],
): string | null {
  let lastStart: string | null = null;
  for (const line of lines) {
    if (line.event_type === "tool_call_start") {
      const fromRaw = line.raw.match(/name=([^\s]+)/)?.[1];
      lastStart =
        fromRaw ||
        line.title?.replace(/\s+started$/i, "") ||
        line.title ||
        null;
    }
    if (line.event_type === "tool_call_end") {
      lastStart = null;
    }
  }
  return lastStart;
}
