import { useEffect, useMemo, useState } from "react";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";

const LONG_WAIT_SECONDS = 15;
const VERY_LONG_WAIT_SECONDS = 30;

type Props = {
  /** Live agent progress / narration stream (auto-updating). */
  progressText?: string | null;
  startedAt: string | null;
  /** Show pill while the turn is active even before the first progress line. */
  running?: boolean;
};

/**
 * Compact status pill after「提交并推送」: spinner + live progress text + elapsed.
 * Not phase labels like「正在执行工具」/「正在处理」.
 */
export function TurnPhaseBanner({
  progressText = null,
  startedAt,
  running = false,
}: Props) {
  const t = useT();
  const [now, setNow] = useState(() => Date.now());
  const [fallbackStartedAt] = useState(() => new Date().toISOString());

  const clockStart = startedAt ?? (running ? fallbackStartedAt : null);
  const trimmed = progressText?.trim() || "";

  useEffect(() => {
    if (!running && !trimmed) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [running, trimmed]);

  const elapsedSeconds = useMemo(() => {
    if (!clockStart) return 0;
    const start = Date.parse(
      clockStart.includes("T") ? clockStart : clockStart.replace(" ", "T"),
    );
    if (Number.isNaN(start)) return 0;
    return Math.max(0, Math.floor((now - start) / 1000));
  }, [now, clockStart]);

  if (!running && !trimmed) return null;

  const label = trimmed || t("conversations.thinkingRunning");

  const longWait = elapsedSeconds >= LONG_WAIT_SECONDS;
  const veryLongWait = elapsedSeconds >= VERY_LONG_WAIT_SECONDS;
  const hint = veryLongWait
    ? t("conversations.turnPhaseTakingLonger")
    : longWait
      ? t("conversations.turnPhaseStillWorking")
      : label;

  return (
    <div
      className={`conv-git-bar__pill conv-git-bar__pill--phase conv-git-bar__pill--progress ${
        veryLongWait ? "conv-git-bar__pill--phase-warn" : ""
      }`}
      role="status"
      aria-live="polite"
      title={hint}
      data-testid="turn-progress-pill"
    >
      <Icon
        name={veryLongWait ? "hourglass_empty" : "progress_activity"}
        size={14}
        className={
          veryLongWait ? "text-warn shrink-0" : "text-primary animate-spin shrink-0"
        }
      />
      <span className="conv-git-bar__progress-text">{label}</span>
      <span className="conv-git-bar__phase-elapsed tabular-nums shrink-0">
        {elapsedSeconds}s
      </span>
    </div>
  );
}
