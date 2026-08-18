/**
 * In-place desktop auto-update (Tauri updater plugin, official-site endpoint).
 *
 * Follows the desktopShell.ts pattern: everything is a no-op outside the
 * Tauri desktop shell, and plugin modules are loaded via dynamic import so
 * the browser build never resolves them.
 */
import { isTauriDesktop } from "@/lib/desktopShell";

export interface AvailableDesktopUpdate {
  version: string;
  notes?: string;
  date?: string;
}

type UpdaterPlugin = typeof import("@tauri-apps/plugin-updater");
type Update = Awaited<ReturnType<UpdaterPlugin["check"]>>;

/** The Update handle from the last successful check; kept for downloadAndInstall. */
let pendingUpdate: Update = null;

function updaterEnabled(): boolean {
  // Skip in `tauri dev` / vite dev to avoid noisy failed checks against prod.
  if (import.meta.env.DEV) return false;
  return isTauriDesktop();
}

/**
 * Check the official-site manifest for a newer version.
 * Returns null when not desktop, already up-to-date, or on any failure
 * (network/portal issues must never break the app).
 */
export async function checkForDesktopUpdate(): Promise<AvailableDesktopUpdate | null> {
  pendingUpdate = null;
  if (!updaterEnabled()) return null;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) return null;
    pendingUpdate = update;
    return {
      version: update.version,
      notes: update.body ?? undefined,
      date: update.date ?? undefined,
    };
  } catch (err) {
    console.warn("[desktop-updater] check failed:", err);
    return null;
  }
}

/**
 * Download and install the update found by the last check.
 * Throws on failure so callers can surface an error state.
 */
export async function downloadAndInstallDesktopUpdate(
  onProgress?: (downloaded: number, total: number | undefined) => void,
): Promise<void> {
  if (!updaterEnabled() || !pendingUpdate) {
    throw new Error("no_pending_update");
  }
  let downloaded = 0;
  await pendingUpdate.downloadAndInstall((event) => {
    if (event.event === "Started") {
      onProgress?.(0, event.data.contentLength);
    } else if (event.event === "Progress") {
      downloaded += event.data.chunkLength;
      onProgress?.(downloaded, undefined);
    } else if (event.event === "Finished") {
      // install follows; nothing to report
    }
  });
  pendingUpdate = null;
  await relaunchDesktopApp();
}

/** Restart the app (runs the normal exit cleanup: dashboard + CEF shutdown). */
export async function relaunchDesktopApp(): Promise<void> {
  if (!isTauriDesktop()) return;
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
