import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { BrowserHitTestResult, ScreencastMetadata } from "@/api/types/workbench";
import { systemBrowserViewport } from "@/lib/browserViewport";
import { cefBrowserStatus } from "@/lib/cefBrowserEmbed";
import { isTauriDesktop } from "@/lib/desktopShell";

function parseApiError(err: unknown): string {
  if (!(err instanceof Error)) return String(err);
  const raw = err.message;
  try {
    const jsonStart = raw.indexOf("{");
    if (jsonStart >= 0) {
      const body = JSON.parse(raw.slice(jsonStart)) as { error?: string; message?: string };
      return body.error ?? body.message ?? raw;
    }
  } catch {
    /* keep raw */
  }
  return raw;
}

export const DEFAULT_VIEWPORT = { width: 1920, height: 1080 };

const HOVER_HIT_THROTTLE_MS = 90;

function mapClientToDevice(
  clientX: number,
  clientY: number,
  img: HTMLImageElement,
  deviceW: number,
  deviceH: number,
): { x: number; y: number; relX: number; relY: number } | null {
  const rect = img.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  const relX = (clientX - rect.left) / rect.width;
  const relY = (clientY - rect.top) / rect.height;
  if (relX < 0 || relX > 1 || relY < 0 || relY > 1) return null;
  const x = Math.max(0, Math.min(deviceW - 1, relX * deviceW));
  const y = Math.max(0, Math.min(deviceH - 1, relY * deviceH));
  return { x, y, relX, relY };
}

export function useWorkbenchBrowser(
  projectId: string | null | undefined,
  conversationSessionId: string | null | undefined,
  active: boolean,
  /**
   * Desktop CEF (default): wait for the native surface + CDP port before
   * creating a session so Browser* attach to the embedded view. Set
   * ANYCODE_CEF_EMBED=0 to use headless + screencast instead.
   */
  opts: { preferCef?: boolean; cefSurfaceReady?: boolean } = {},
) {
  const preferCef = Boolean(opts.preferCef);
  const cefSurfaceReady = Boolean(opts.cefSurfaceReady);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [urlInput, setUrlInput] = useState("about:blank");
  const [lockState, setLockState] = useState<string>("idle");
  const [frameBase64, setFrameBase64] = useState<string | null>(null);
  const [frameMeta, setFrameMeta] = useState<ScreencastMetadata | null>(null);
  const [pickMode, setPickMode] = useState(false);
  const [picked, setPicked] = useState<BrowserHitTestResult | null>(null);
  const [hoverHit, setHoverHit] = useState<BrowserHitTestResult | null>(null);
  const [pickPending, setPickPending] = useState(false);
  const sessionRef = useRef<string | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const frameMetaRef = useRef<ScreencastMetadata | null>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const hoverInFlightRef = useRef(false);
  const [createError, setCreateError] = useState<Error | null>(null);
  const [navPending, setNavPending] = useState(false);

  const status = useQuery({
    queryKey: ["workbench-browser-status"],
    queryFn: api.browserStatus,
    enabled: active,
    staleTime: 30_000,
  });

  const [cefReady, setCefReady] = useState(false);
  const chromiumReady = status.data?.ready ?? status.data?.chromium_ready ?? false;
  const browserEnabled = status.data?.enabled ?? false;
  // Desktop CEF can satisfy readiness even before headless Chromium is resolved.
  const canUseBrowser =
    chromiumReady ||
    (preferCef && (cefReady || cefSurfaceReady)) ||
    (isTauriDesktop() && cefReady);
  // Prefer attaching after the CEF NSView exists so we don't spawn headless.
  const shouldCreateSession =
    active &&
    Boolean(projectId) &&
    canUseBrowser &&
    (!preferCef || cefSurfaceReady);
  // Native CEF preview when preferred and surface/port ready.
  const useCefPreview = preferCef && (cefSurfaceReady || cefReady);

  const deviceWidth = frameMeta?.device_width || DEFAULT_VIEWPORT.width;
  const deviceHeight = frameMeta?.device_height || DEFAULT_VIEWPORT.height;

  useEffect(() => {
    frameMetaRef.current = frameMeta;
  }, [frameMeta]);

  useEffect(() => {
    if (!preferCef || !isTauriDesktop() || !active) {
      setCefReady(false);
      return;
    }
    let cancelled = false;
    const probe = () => {
      void cefBrowserStatus().then((s) => {
        if (!cancelled) setCefReady(Boolean(s?.ready && (s.remote_debugging_port ?? 0) > 0));
      });
    };
    probe();
    const id = window.setInterval(probe, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [active, preferCef]);

  useEffect(() => {
    if (!shouldCreateSession) {
      setCreateError(null);
      return;
    }
    let cancelled = false;
    setCreateError(null);

    const create = async () => {
      // Must have CEF CDP port before create — otherwise attach fails hard
      // (no silent headless) and Agent would not share the live view.
      if (preferCef && isTauriDesktop()) {
        let gotPort = false;
        for (let i = 0; i < 40 && !cancelled; i++) {
          const s = await cefBrowserStatus();
          if (s?.ready && (s.remote_debugging_port ?? 0) > 0) {
            setCefReady(true);
            gotPort = true;
            break;
          }
          await new Promise((r) => window.setTimeout(r, 100));
        }
        if (!gotPort) {
          if (!cancelled) {
            setCreateError(
              new Error("CEF CDP port not ready — keep the Browser panel open and retry"),
            );
          }
          return;
        }
      }
      if (cancelled) return;
      try {
        const data = await api.createBrowserSession(
          projectId!,
          conversationSessionId ?? undefined,
          systemBrowserViewport(),
        );
        if (cancelled) return;
        sessionRef.current = data.session.session_id;
        setSessionId(data.session.session_id);
      } catch (e) {
        if (!cancelled) setCreateError(new Error(parseApiError(e)));
      }
    };
    void create();

    return () => {
      cancelled = true;
      sessionRef.current = null;
      setSessionId(null);
      setFrameBase64(null);
      setFrameMeta(null);
      setPicked(null);
      setHoverHit(null);
      setPickMode(false);
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [shouldCreateSession, projectId, conversationSessionId, cefSurfaceReady, preferCef]);

  useEffect(() => {
    if (!active || !sessionId) return;
    const poll = () => {
      void api.browserState(sessionId).then((r) => {
        if (r.state.lock) setLockState(r.state.lock);
        // With native CEF embed, the panel owns the address bar — CDP state
        // must not fight the user while they type (or while CEF syncs).
        if (r.state.url && !useCefPreview) setUrlInput(r.state.url);
      });
    };
    poll();
    const id = window.setInterval(poll, 1500);
    return () => window.clearInterval(id);
  }, [active, sessionId, useCefPreview]);

  // Screencast JPEG preview is for headless/daemon only — CEF shows a real view.
  useEffect(() => {
    if (!active || !sessionId || useCefPreview) {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
      return;
    }
    const ws = new WebSocket(api.browserStreamUrl(sessionId));
    wsRef.current = ws;
    ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(String(ev.data)) as {
          image_base64?: string;
          format?: string;
          metadata?: ScreencastMetadata;
        };
        if (data.image_base64) {
          const mime = data.format === "jpeg" ? "image/jpeg" : "image/png";
          setFrameBase64(`${mime}:${data.image_base64}`);
        }
        if (data.metadata) {
          setFrameMeta(data.metadata);
        }
      } catch {
        /* ignore */
      }
    };
    return () => {
      ws.close();
      if (wsRef.current === ws) wsRef.current = null;
    };
  }, [active, sessionId, useCefPreview]);

  useEffect(() => {
    return () => {
      if (hoverTimerRef.current != null) {
        window.clearTimeout(hoverTimerRef.current);
      }
    };
  }, []);

  const navigate = {
    isPending: navPending,
    mutate: (url: string) => {
      const sid = sessionRef.current;
      if (!sid) return;
      setNavPending(true);
      void api
        .browserLock(sid, "user")
        .then(() => api.navigateBrowser(sid, url))
        .then((result) => {
          setUrlInput(result.state.url);
          setLockState(result.state.lock ?? "user");
        })
        .finally(() => setNavPending(false));
    },
  };

  const refresh = () => {
    const url = urlInput.trim();
    if (!url || url === "about:blank") return;
    // CEF preview refreshes via cefBrowserNavigate in BrowserPanel.
    // Default screencast path uses CDP navigate so the panel tracks Agent.
    if (useCefPreview) return;
    navigate.mutate(url);
  };

  const unlockForUser = () => {
    const sid = sessionRef.current;
    if (!sid) return;
    void api.browserLock(sid, "user").then((r) => setLockState(r.lock));
  };

  const hitTestAtImagePoint = useCallback(
    (clientX: number, clientY: number, img: HTMLImageElement) => {
      const sid = sessionRef.current;
      if (!sid || pickPending) return;
      const meta = frameMetaRef.current;
      const deviceW = meta?.device_width || DEFAULT_VIEWPORT.width;
      const deviceH = meta?.device_height || DEFAULT_VIEWPORT.height;
      const mapped = mapClientToDevice(clientX, clientY, img, deviceW, deviceH);
      if (!mapped) return;
      setPickPending(true);
      void api
        .browserHitTest(sid, mapped.x, mapped.y)
        .then((r) => {
          if (r.hit) setPicked(r.hit);
        })
        .finally(() => setPickPending(false));
    },
    [pickPending],
  );

  /** Throttled hover hit-test for Design inspect mode (does not set picked). */
  const hoverHitTestAtImagePoint = useCallback(
    (clientX: number, clientY: number, img: HTMLImageElement) => {
      const sid = sessionRef.current;
      if (!sid) return;
      const meta = frameMetaRef.current;
      const deviceW = meta?.device_width || DEFAULT_VIEWPORT.width;
      const deviceH = meta?.device_height || DEFAULT_VIEWPORT.height;
      const mapped = mapClientToDevice(clientX, clientY, img, deviceW, deviceH);
      if (!mapped) {
        setHoverHit(null);
        return;
      }
      if (hoverTimerRef.current != null) {
        window.clearTimeout(hoverTimerRef.current);
      }
      hoverTimerRef.current = window.setTimeout(() => {
        if (hoverInFlightRef.current) return;
        hoverInFlightRef.current = true;
        void api
          .browserHitTest(sid, mapped.x, mapped.y)
          .then((r) => {
            setHoverHit(r.hit ?? null);
          })
          .finally(() => {
            hoverInFlightRef.current = false;
          });
      }, HOVER_HIT_THROTTLE_MS);
    },
    [],
  );

  const clearHoverHit = useCallback(() => {
    setHoverHit(null);
    if (hoverTimerRef.current != null) {
      window.clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  /** CEF Design: inject/remove in-page inspect (React overlays cannot sit above CEF). */
  const setDesignInspect = useCallback(
    async (enabled: boolean) => {
      const sid = sessionRef.current;
      if (!sid || !useCefPreview) return;
      try {
        await api.browserDesignMode(sid, enabled);
        if (!enabled) {
          setHoverHit(null);
        }
      } catch {
        /* session may not be ready yet */
      }
    },
    [useCefPreview],
  );

  useEffect(() => {
    if (!active || !sessionId || !pickMode || !useCefPreview) return;
    const tick = () => {
      void api
        .browserDesignInspect(sessionId)
        .then((r) => {
          if (r.inspect.hover) setHoverHit(r.inspect.hover);
          else setHoverHit(null);
          if (r.inspect.pick) setPicked(r.inspect.pick);
        })
        .catch(() => {
          /* ignore transient CDP errors */
        });
    };
    tick();
    const id = window.setInterval(tick, 70);
    return () => window.clearInterval(id);
  }, [active, sessionId, pickMode, useCefPreview]);

  const urlTrimmed = urlInput.trim();
  const hasMeaningfulUrl =
    urlTrimmed.length > 0 &&
    urlTrimmed !== "about:blank" &&
    urlTrimmed !== "https://example.com";

  return {
    urlInput,
    setUrlInput,
    navigate,
    refresh,
    lockState,
    unlockForUser,
    hasMeaningfulUrl,
    pickMode,
    setPickMode,
    setDesignInspect,
    picked,
    setPicked,
    hoverHit,
    clearHoverHit,
    pickPending,
    hitTestAtImagePoint,
    hoverHitTestAtImagePoint,
    deviceWidth,
    deviceHeight,
    frameMeta,
    screenshot: {
      data: frameBase64
        ? {
            screenshot: {
              image_base64: frameBase64.includes(":")
                ? frameBase64.split(":").slice(1).join(":")
                : frameBase64,
              mime: frameBase64.startsWith("image/jpeg:") ? "image/jpeg" : "image/png",
            },
          }
        : null,
    },
    createSession: {
      isPending: shouldCreateSession && !sessionId && !createError,
      isError: Boolean(createError),
      error: createError,
    },
    sessionReady: Boolean(sessionId),
    status: {
      isLoading: status.isLoading,
      chromiumReady: canUseBrowser,
      browserEnabled,
      canUseBrowser,
      cefReady,
      doctorMessage: status.data?.doctor_message,
    },
  };
}
