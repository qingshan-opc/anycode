import type { TranscriptBlock } from "@/api/types";
import {
  browserToolDedupeKey,
  collectBrowserToolCallKeys,
  extractBrowserNavigateUrl,
  isBrowserToolBlock,
  shouldMirrorNavigateToWorkbench,
} from "@/lib/browserToolDetect";

/**
 * Tracks Browser tool blocks for one conversation session and decides when the
 * Browser workbench panel must auto-open.
 *
 * Timing contract (CEF attach chain): a *live* Browser tool call that arrives
 * after hydration opens the panel so the embedded CEF creates a page that
 * Agent Browser* tools attach to. Everything already present at hydration is
 * indexed silently — re-entering a session mid-stream must never re-open a
 * panel the user closed; history replay is likewise silent.
 */
export class BrowserAutoOpenTracker {
  private hydrated = false;
  private seen = new Set<string>();

  reset(): void {
    this.hydrated = false;
    this.seen = new Set();
  }

  /**
   * Ingest the current live blocks. Returns true whenever a not-yet-seen live
   * Browser tool_call appears (callers should treat openTab as idempotent).
   */
  ingest(blocks: TranscriptBlock[], streamLive: boolean): boolean {
    if (!this.hydrated) {
      this.hydrated = true;
      // Always index existing blocks silently — including when the stream is
      // already live (session re-entry). Only calls arriving *after* this
      // point count as new; otherwise every session click during a live
      // stream would re-open the panel the user just closed.
      this.seen = collectBrowserToolCallKeys(blocks);
      return false;
    }
    if (!streamLive) {
      // History replay / pagination: index silently, never auto-open.
      for (const key of collectBrowserToolCallKeys(blocks)) {
        this.seen.add(key);
      }
      return false;
    }
    for (const call of blocks) {
      if (call.block_type !== "tool_call" || !isBrowserToolBlock(call)) continue;
      const key = browserToolDedupeKey(call);
      if (this.seen.has(key)) continue;
      this.seen.add(key);
      return true;
    }
    return false;
  }
}

/**
 * Dedupes MCP / external navigate URLs mirrored into the shared workbench CDP
 * session so the right panel shows the same page (Playwright MCP otherwise
 * stays isolated).
 */
export class NavigateMirrorTracker {
  private hydrated = false;
  private seen = new Set<string>();

  reset(): void {
    this.hydrated = false;
    this.seen = new Set();
  }

  /**
   * Returns the first not-yet-mirrored navigate URL in `blocks`, or null.
   * Deduped by URL — tool_call + tool_result must not navigate twice.
   * The first ingest after a reset only indexes: re-entering a session
   * mid-stream must not re-navigate (and re-open the panel) for URLs the
   * agent visited before the user came back.
   */
  ingest(blocks: TranscriptBlock[]): string | null {
    const collect = (): Set<string> => {
      const urls = new Set<string>();
      for (const block of blocks) {
        if (!shouldMirrorNavigateToWorkbench(block)) continue;
        const url = extractBrowserNavigateUrl(block);
        if (!url || url === "about:blank") continue;
        urls.add(url);
      }
      return urls;
    };
    if (!this.hydrated) {
      this.hydrated = true;
      this.seen = collect();
      return null;
    }
    for (const block of blocks) {
      if (!shouldMirrorNavigateToWorkbench(block)) continue;
      const url = extractBrowserNavigateUrl(block);
      if (!url || url === "about:blank") continue;
      if (this.seen.has(url)) continue;
      this.seen.add(url);
      return url;
    }
    return null;
  }
}

/**
 * Tracks plan-tree revisions for one session: a new revision (updated_at
 * change after the initial hydrate) opens the Plan panel for human review.
 */
export class PlanAutoOpenTracker {
  private hydrated = false;
  private lastKey: string | null = null;

  reset(): void {
    this.hydrated = false;
    this.lastKey = null;
  }

  /** Returns true when a new plan revision arrives after the initial hydrate. */
  ingest(updatedAt: string | null, rootsCount: number): boolean {
    if (rootsCount === 0 || !updatedAt) {
      this.lastKey = null;
      this.hydrated = false;
      return false;
    }
    if (!this.hydrated) {
      this.hydrated = true;
      this.lastKey = updatedAt;
      return false;
    }
    if (this.lastKey === updatedAt) return false;
    this.lastKey = updatedAt;
    return true;
  }
}
