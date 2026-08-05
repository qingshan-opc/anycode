import { useEffect, useRef } from "react";
import { api } from "@/api/client";
import type { TranscriptBlock } from "@/api/types";
import type { WorkbenchTab } from "@/api/types/workbench";
import { systemBrowserViewport } from "@/lib/browserViewport";
import { cefBrowserStatus } from "@/lib/cefBrowserEmbed";
import { isTauriDesktop } from "@/lib/desktopShell";
import { BrowserAutoOpenTracker, NavigateMirrorTracker } from "@/lib/workbenchAutoOpen";

export type BrowserAutoOpenCtx = {
  sessionId: string | null;
  projectId: string | null;
  liveBlocks: TranscriptBlock[] | null;
  streamLive: boolean;
  openTab: (tab: WorkbenchTab) => void;
};

/**
 * Browser panel auto-open + navigate mirroring.
 *
 * Auto-open: a *live* Browser tool call must always open the panel so the
 * embedded CEF creates a page that Agent Browser* tools attach to; history
 * replay is indexed silently (see BrowserAutoOpenTracker).
 *
 * Mirroring: MCP / external navigate URLs are replayed into the shared
 * workbench CDP session so the panel shows the same page (Playwright MCP
 * otherwise stays isolated).
 */
export function useBrowserAutoOpen({
  sessionId,
  projectId,
  liveBlocks,
  streamLive,
  openTab,
}: BrowserAutoOpenCtx): void {
  const autoOpenRef = useRef(new BrowserAutoOpenTracker());
  const mirrorRef = useRef(new NavigateMirrorTracker());

  useEffect(() => {
    autoOpenRef.current.reset();
    mirrorRef.current.reset();
  }, [sessionId]);

  useEffect(() => {
    if (!sessionId) return;
    if (autoOpenRef.current.ingest(liveBlocks ?? [], streamLive)) {
      openTab("browser");
    }
  }, [sessionId, liveBlocks, streamLive, openTab]);

  useEffect(() => {
    if (!sessionId || !projectId || !streamLive) return;
    const url = mirrorRef.current.ingest(liveBlocks ?? []);
    if (!url) return;
    openTab("browser");
    void (async () => {
      try {
        if (isTauriDesktop()) {
          // Wait for CEF to publish CDP port so mirror navigates the live view.
          let ready = false;
          for (let i = 0; i < 40; i++) {
            const s = await cefBrowserStatus();
            if (s?.ready && (s.remote_debugging_port ?? 0) > 0) {
              ready = true;
              break;
            }
            await new Promise((r) => window.setTimeout(r, 100));
          }
          if (!ready) return;
        }
        const created = await api.createBrowserSession(
          projectId,
          sessionId,
          systemBrowserViewport(),
        );
        await api.navigateBrowser(created.session.session_id, url);
      } catch {
        /* panel poll / agent may still update; ignore mirror failures */
      }
    })();
  }, [sessionId, projectId, liveBlocks, streamLive, openTab]);
}
