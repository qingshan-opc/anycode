import { isTauriDesktop } from "@/lib/desktopShell";

export type CefTabInfo = {
  id: number;
  url: string;
  title: string;
  active: boolean;
};

export type CefEmbedStatus = {
  ready: boolean;
  remote_debugging_port: number;
  url?: string | null;
  title?: string | null;
  tabs?: CefTabInfo[];
  active_tab_id?: number | null;
};

async function invokeTauri<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

export async function cefBrowserStatus(): Promise<CefEmbedStatus | null> {
  if (!isTauriDesktop()) return null;
  try {
    return await invokeTauri<CefEmbedStatus>("cef_browser_status");
  } catch (e) {
    console.warn("[cef] cef_browser_status failed", e);
    return null;
  }
}

export async function cefBrowserShow(
  rect: { x: number; y: number; width: number; height: number },
  url: string,
): Promise<CefEmbedStatus> {
  return invokeTauri<CefEmbedStatus>("cef_browser_show", {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
    url,
  });
}

export async function cefBrowserResize(rect: {
  x: number;
  y: number;
  width: number;
  height: number;
}): Promise<void> {
  await invokeTauri("cef_browser_resize", rect);
}

export async function cefBrowserHide(): Promise<void> {
  if (!isTauriDesktop()) return;
  try {
    await invokeTauri("cef_browser_hide");
  } catch {
    /* ignore */
  }
}

export async function cefBrowserNavigate(url: string): Promise<CefEmbedStatus> {
  return invokeTauri<CefEmbedStatus>("cef_browser_navigate", { url });
}

export async function cefBrowserNewTab(url?: string): Promise<CefEmbedStatus> {
  return invokeTauri<CefEmbedStatus>("cef_browser_new_tab", { url: url ?? null });
}

export async function cefBrowserSelectTab(id: number): Promise<CefEmbedStatus> {
  return invokeTauri<CefEmbedStatus>("cef_browser_select_tab", { id });
}

export async function cefBrowserCloseTab(id: number): Promise<CefEmbedStatus> {
  return invokeTauri<CefEmbedStatus>("cef_browser_close_tab", { id });
}
