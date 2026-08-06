import { useEffect, useRef, useState } from "react";
import { useT } from "@/i18n/context";
import {
  checkForDesktopUpdate,
  downloadAndInstallDesktopUpdate,
  relaunchDesktopApp,
  type AvailableDesktopUpdate,
} from "@/lib/desktopUpdater";
import { isTauriDesktop } from "@/lib/desktopShell";

type BannerState =
  | { kind: "idle" }
  | { kind: "available"; update: AvailableDesktopUpdate }
  | { kind: "downloading"; percent: number | null }
  | { kind: "ready" }
  | { kind: "error"; message: string };

const STARTUP_CHECK_DELAY_MS = 3_000;

/**
 * Startup auto-update banner (desktop only): silent check shortly after
 * mount; on a newer version offers "update now" → download/install with
 * progress → relaunch prompt. Dismissal is session-scoped (re-mounting
 * Layout within the session won't re-check because of the module flag below).
 */
let sessionDismissed = false;

export function DesktopUpdateBanner() {
  const t = useT();
  const [state, setState] = useState<BannerState>({ kind: "idle" });
  const totalRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!isTauriDesktop() || sessionDismissed) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      void checkForDesktopUpdate().then((update) => {
        if (cancelled || !update || sessionDismissed) return;
        setState({ kind: "available", update });
      });
    }, STARTUP_CHECK_DELAY_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);

  if (!isTauriDesktop() || state.kind === "idle") return null;

  const dismiss = () => {
    sessionDismissed = true;
    setState({ kind: "idle" });
  };

  const startUpdate = async () => {
    totalRef.current = undefined;
    setState({ kind: "downloading", percent: null });
    try {
      await downloadAndInstallDesktopUpdate((downloaded, total) => {
        if (total !== undefined) totalRef.current = total;
        const knownTotal = totalRef.current;
        setState({
          kind: "downloading",
          percent:
            knownTotal && knownTotal > 0
              ? Math.min(100, Math.round((downloaded / knownTotal) * 100))
              : null,
        });
      });
      setState({ kind: "ready" });
    } catch (err) {
      setState({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  return (
    <div
      role="status"
      className="fixed top-3 left-1/2 -translate-x-1/2 z-[400] flex items-center gap-3 rounded-xl border border-outline-variant bg-surface-container-low px-4 py-2.5 shadow-lg"
    >
      {state.kind === "available" && (
        <>
          <span className="text-sm text-on-surface">
            {t("update.bannerTitle").replace("{version}", state.update.version)}
          </span>
          <button
            type="button"
            className="dw-btn-secondary text-xs"
            onClick={() => void startUpdate()}
          >
            {t("update.updateNow")}
          </button>
          <button
            type="button"
            className="text-xs text-secondary hover:text-on-surface"
            onClick={dismiss}
          >
            {t("update.later")}
          </button>
        </>
      )}
      {state.kind === "downloading" && (
        <span className="text-sm text-on-surface">
          {state.percent !== null
            ? t("update.downloading").replace("{percent}", String(state.percent))
            : t("update.downloading").replace("{percent}", "…")}
        </span>
      )}
      {state.kind === "ready" && (
        <>
          <span className="text-sm text-on-surface">{t("update.ready")}</span>
          <button
            type="button"
            className="dw-btn-secondary text-xs"
            onClick={() => void relaunchDesktopApp()}
          >
            {t("update.relaunch")}
          </button>
        </>
      )}
      {state.kind === "error" && (
        <>
          <span className="text-sm text-error">
            {t("update.failed").replace("{message}", state.message)}
          </span>
          <button
            type="button"
            className="text-xs text-secondary hover:text-on-surface"
            onClick={dismiss}
          >
            {t("update.later")}
          </button>
        </>
      )}
    </div>
  );
}
