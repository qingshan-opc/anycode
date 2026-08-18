import { memo, useEffect, useMemo, useState } from "react";
import type { TranscriptBlock } from "@/api/types";
import { Icon } from "@/components/Icon";
import { AgentActivityLine } from "@/components/chat/AgentActivityLine";
import { formatTranscriptBlockTitle } from "@/lib/eventFormat";
import {
  countLogicalToolSteps,
  toolStepFailed,
  toolStepRunning,
  type ToolStep,
} from "@/lib/transcriptGrouping";
import {
  formatAgentActivityRecap,
  truncateThinkingPreview,
} from "@/lib/agentActivitySummary";
import {
  toolTraceShowThinkingHeader,
  toolTraceStreaming,
} from "@/lib/toolTraceState";
import {
  extractToolCommand,
  formatDurationMs,
  formatToolStepLabelParts,
} from "@/lib/toolStepLabel";
import { toolIconName } from "@/lib/toolIcons";
import { useT } from "@/i18n/context";
import { useSmoothText } from "@/hooks/useSmoothText";
import { formatDeliveryPreflight } from "@/lib/progressMeta";

type Props = {
  steps: ToolStep[];
  processMessageCount?: number;
  processSnippets?: string[];
  isRunning?: boolean;
  selectedToolId?: string | null;
  onSelectTool?: (tool: TranscriptBlock) => void;
  suppressActivityLine?: boolean;
  defaultCollapsed?: boolean;
  /** Accordion: force open/closed from parent (overrides local history toggle when set). */
  forceExpanded?: boolean;
  /** Settled (non-streaming) clusters: render step rows instead of a one-line summary. */
  showSettledSteps?: boolean;
  variant?: "nested" | "flat";
};

export const ToolTraceCluster = memo(function ToolTraceCluster({
  steps,
  processMessageCount = 0,
  processSnippets = [],
  isRunning = false,
  selectedToolId,
  onSelectTool,
  suppressActivityLine = false,
  defaultCollapsed: _defaultCollapsed = true,
  forceExpanded,
  showSettledSteps = false,
  variant = "nested",
}: Props) {
  const t = useT();
  const [historyOpen, setHistoryOpen] = useState(false);
  const anyFailed = steps.some(toolStepFailed);
  const thinkingSignal = Math.max(processMessageCount, processSnippets.length);
  const streaming = toolTraceStreaming(steps, thinkingSignal, isRunning);

  const summary = useMemo(() => {
    const count = countLogicalToolSteps(
      steps.flatMap((step) => [step.call, step.result].filter(Boolean) as TranscriptBlock[]),
    );
    const last = [...steps].reverse().find((step) => step.call || step.result);
    const lastLabel = last
      ? formatToolStepLabelParts(
          last,
          (block) => formatTranscriptBlockTitle(block, t),
          { segmentActive: isRunning },
        ).toolName
      : "";
    const totalMs = steps.reduce((acc, step) => {
      const meta = (step.result?.meta ?? step.call?.meta ?? {}) as Record<string, unknown>;
      const raw = meta.duration_ms ?? meta.elapsed_ms;
      const ms =
        typeof raw === "string"
          ? Number.parseInt(raw, 10)
          : typeof raw === "number"
            ? raw
            : 0;
      return acc + (Number.isNaN(ms) ? 0 : ms);
    }, 0);
    return { count, lastLabel, totalDuration: formatDurationMs({ duration_ms: totalMs }) };
  }, [steps, t, isRunning]);

  const activityRecap = useMemo(
    () => formatAgentActivityRecap(steps, t),
    [steps, t],
  );

  const toolUsageLine = useMemo(() => {
    if (summary.count <= 0) return null;
    if (anyFailed) {
      return t("conversations.toolTraceFailed").replace("{n}", String(summary.count));
    }
    // Cursor-style settled pill: activity recap only ("Explored 13 files…"),
    // not "Used N tools · Bash".
    if (activityRecap) {
      return activityRecap;
    }
    return t("conversations.toolUsageSummary").replace("{n}", String(summary.count));
  }, [summary.count, anyFailed, activityRecap, t]);

  if (steps.length === 0 && processMessageCount === 0) {
    return null;
  }

  // Settled cluster carrying only folded empty notices (no steps, no thinking
  // snippets) has nothing to render — avoid an empty tool-strip pill.
  if (steps.length === 0 && processSnippets.length === 0 && !streaming) {
    return null;
  }

  const stripClass = anyFailed
    ? "tool-strip--failed"
    : streaming
      ? "tool-strip--running"
      : "tool-strip--done";

  // Streaming: step rows while live. Settled: Cursor pill summary; expand on click
  // or when parent asks for history detail (showSettledSteps).
  const showStepDetails = streaming
    ? forceExpanded !== false
    : Boolean(showSettledSteps || historyOpen);

  if (variant === "flat") {
    if (steps.length === 0) return null;
    return (
      <div className="tool-strip-flat" data-testid="tool-strip-flat">
        {steps.map((step) => (
          <ToolStepLine
            key={step.key}
            step={step}
            selectedToolId={selectedToolId}
            onSelectTool={onSelectTool}
            allowExpand={false}
            segmentActive={isRunning}
          />
        ))}
      </div>
    );
  }

  if (streaming) {
    const showThinkingHeader = toolTraceShowThinkingHeader(
      steps,
      thinkingSignal,
      isRunning,
    );
    const anyToolRunning = steps.some(toolStepRunning);
    // Codex/Cursor: reasoning/prose first, tool cards under it — never claim a
    // finished tool is still "running" just because the tip cluster is live.
    return (
      <div className="flex flex-col gap-1.5 w-full max-w-[var(--conv-content-max)]">
        {!suppressActivityLine && steps.length > 0 && (
          <AgentActivityLine steps={steps} suppressDuration />
        )}
        <div className={`tool-strip tool-strip-streaming ${stripClass}`}>
          {showThinkingHeader && (
            <ThinkingTraceFold
              count={thinkingSignal}
              snippets={processSnippets}
              loading
            />
          )}
          {anyToolRunning && (
            <div className="tool-strip-summary" role="status">
              <Icon name="progress_activity" size={14} className="text-primary animate-spin" />
              <span>
                {summary.lastLabel
                  ? t("conversations.toolTraceRunningTool").replace(
                      "{tool}",
                      summary.lastLabel,
                    )
                  : t("conversations.agentWorking")}
              </span>
            </div>
          )}
          {showStepDetails &&
            steps.map((step) => (
              <ToolStepLine
                key={step.key}
                step={step}
                selectedToolId={selectedToolId}
                onSelectTool={onSelectTool}
                allowExpand={false}
                segmentActive={isRunning}
              />
            ))}
        </div>
      </div>
    );
  }

  const showThinkingFold = thinkingSignal > 0 && processSnippets.length > 0;

  return (
    <div className="agent-trace-meta w-full max-w-[var(--conv-content-max)]">
      {showThinkingFold && (
        <ThinkingTraceFold
          count={thinkingSignal}
          snippets={processSnippets}
          loading={false}
          settled
        />
      )}
      {toolUsageLine && (
        <button
          type="button"
          className={`agent-trace-meta__pill ${historyOpen || showSettledSteps ? "agent-trace-meta__pill--open" : ""}`}
          data-testid="tool-usage-summary"
          aria-expanded={showStepDetails}
          onClick={() => setHistoryOpen((v) => !v)}
        >
          <Icon
            name={showStepDetails ? "expand_more" : "chevron_right"}
            size={16}
            className="transcript-expand-icon shrink-0"
          />
          <span className="agent-trace-meta__line m-0">
            {toolUsageLine}
            {summary.totalDuration ? (
              <span className="agent-trace-meta__meta"> · {summary.totalDuration}</span>
            ) : null}
          </span>
        </button>
      )}
      {showStepDetails && steps.length > 0 && (
        <div className={`tool-strip tool-strip-settled ${stripClass}`}>
          {steps.map((step) => (
            <ToolStepLine
              key={step.key}
              step={step}
              selectedToolId={selectedToolId}
              onSelectTool={onSelectTool}
              allowExpand
              segmentActive={false}
            />
          ))}
        </div>
      )}
    </div>
  );
}, (prev, next) =>
  prev.steps === next.steps &&
  prev.processMessageCount === next.processMessageCount &&
  prev.processSnippets === next.processSnippets &&
  prev.isRunning === next.isRunning &&
  prev.selectedToolId === next.selectedToolId &&
  prev.onSelectTool === next.onSelectTool &&
  prev.suppressActivityLine === next.suppressActivityLine &&
  prev.defaultCollapsed === next.defaultCollapsed &&
  prev.forceExpanded === next.forceExpanded &&
  prev.showSettledSteps === next.showSettledSteps &&
  prev.variant === next.variant);

function ThinkingTraceFold({
  count,
  snippets,
  loading,
  settled = false,
}: {
  count: number;
  snippets: string[];
  loading: boolean;
  settled?: boolean;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const latestSnippet = snippets[snippets.length - 1] ?? "";
  const { text: smoothedSnippet } = useSmoothText(
    `thinking:${count}:${snippets.length}`,
    latestSnippet,
    loading && latestSnippet.length > 0,
  );
  const previewText = truncateThinkingPreview(
    loading ? smoothedSnippet : latestSnippet,
  );

  useEffect(() => {
    if (loading) {
      setOpen(false);
    }
  }, [loading]);

  if (!loading && snippets.length === 0) {
    return null;
  }

  const label = loading
    ? t("conversations.thinkingRunning")
    : count <= 1
      ? t("conversations.thinkingBrief")
      : t("conversations.thinkingDone").replace("{n}", String(count));

  // Settled folds still show a one-line preview so long context isn't trapped
  // behind "已思考 N 步" with the real text only in the composer pill.
  const showCollapsedPreview = !open && !loading && previewText.length > 0;

  const thinkingStateClass = open
    ? "tool-strip-step-thinking--open"
    : loading && snippets.length > 0 && !settled
      ? "tool-strip-step-thinking--streaming"
      : "";

  return (
    <div
      className={`tool-strip-step tool-strip-step-thinking ${settled ? "tool-strip-step-thinking--settled" : ""} ${thinkingStateClass}`.trim()}
    >
      <button
        type="button"
        className="tool-strip-step-toggle"
        onClick={() => {
          if (!loading && snippets.length > 0) {
            setOpen((v) => !v);
          }
        }}
        aria-expanded={open || showCollapsedPreview}
        aria-label={open ? t("conversations.thinkingCollapse") : t("conversations.thinkingExpand")}
        disabled={loading || snippets.length === 0}
      >
        {!loading && snippets.length > 0 && (
          <Icon
            name={open ? "expand_more" : "chevron_right"}
            size={18}
            className="transcript-expand-icon shrink-0"
          />
        )}
        <span>{label}</span>
      </button>
      {showCollapsedPreview && (
        <p className="tool-strip-step-thinking-preview tool-strip-step-thinking-text">{previewText}</p>
      )}
      {open && !loading && snippets.length > 0 && (
        <p className="tool-strip-step-thinking-preview tool-strip-step-thinking-text">
          {formatDeliveryPreflight(snippets[snippets.length - 1] ?? "") ??
            snippets[snippets.length - 1] ?? ""}
        </p>
      )}
      {loading && snippets.length > 0 && !settled && (
        <p className="tool-strip-step-thinking-preview tool-strip-step-thinking-text">{smoothedSnippet}</p>
      )}
    </div>
  );
}

function ToolStepLine({
  step,
  selectedToolId,
  onSelectTool,
  allowExpand = true,
  segmentActive = true,
}: {
  step: ToolStep;
  selectedToolId?: string | null;
  onSelectTool?: (tool: TranscriptBlock) => void;
  allowExpand?: boolean;
  /** When false, unpaired call-without-result must not spin. */
  segmentActive?: boolean;
}) {
  const t = useT();
  const primary = step.result ?? step.call;
  const unpaired = toolStepRunning(step);
  const running = unpaired && segmentActive;
  const abandoned = unpaired && !segmentActive;
  const failed = toolStepFailed(step) || abandoned;
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    if (running) {
      setExpanded(false);
    }
  }, [running]);

  if (!primary) return null;

  const parts = formatToolStepLabelParts(
    step,
    (block) => formatTranscriptBlockTitle(block, t),
    { segmentActive },
  );
  const toolName =
    (primary.meta?.name as string | undefined) ??
    primary.title.replace(/\s+(started|finished|failed)$/i, "").trim();
  const callBody = extractToolCommand(step.call);
  const resultBody = primary.body?.trim() ?? "";
  const expandBody = [callBody, resultBody].filter(Boolean).join("\n\n");
  const hasBody = expandBody.length > 0;
  const canExpand = allowExpand && !running && hasBody;
  const showBody = canExpand && expanded;

  return (
    <div
      className={`tool-strip-step ${
        running
          ? "tool-strip-step-active"
          : failed
            ? "tool-strip-step-failed"
            : "tool-strip-step-done"
      }`}
    >
      <button
        type="button"
        className="tool-strip-step-toggle"
        onClick={() => {
          if (canExpand) {
            setExpanded((v) => !v);
          }
          onSelectTool?.(primary);
        }}
        aria-expanded={showBody}
        aria-label={expanded ? t("common.collapse") : t("common.expand")}
        data-selected={selectedToolId === primary.id ? "true" : undefined}
        disabled={!canExpand && !onSelectTool}
      >
        {canExpand && (
          <Icon
            name={expanded ? "expand_more" : "chevron_right"}
            size={18}
            className="transcript-expand-icon shrink-0"
          />
        )}
        <Icon
          name={
            failed
              ? "error"
              : running
                ? "progress_activity"
                : toolIconName(toolName)
          }
          size={14}
          className={running ? "animate-spin text-primary" : undefined}
        />
        <span className="tool-strip-step-name">{parts.toolName}</span>
        {parts.command && (
          <span className="tool-strip-step-command font-code">{parts.command}</span>
        )}
        {parts.duration && <span className="tool-strip-step-duration">{parts.duration}</span>}
        {abandoned && (
          <span className="tool-strip-step-duration">
            {t("conversations.toolTraceAbandoned")}
          </span>
        )}
        {!running && (
          <span className="tool-strip-step-status" aria-hidden>
            {failed ? "✗" : "✓"}
          </span>
        )}
      </button>
      {showBody && <pre className="tool-strip-step-body">{expandBody}</pre>}
    </div>
  );
}
