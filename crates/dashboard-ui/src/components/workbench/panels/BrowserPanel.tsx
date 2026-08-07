import { useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";
import { Link } from "@tanstack/react-router";
import {
  chipLabel,
  encodeBrowserDesignMessage,
} from "@/lib/browserDesignMessage";
import {
  cefBrowserCloseTab,
  cefBrowserHide,
  cefBrowserNavigate,
  cefBrowserNewTab,
  cefBrowserResize,
  cefBrowserSelectTab,
  cefBrowserShow,
  cefBrowserStatus,
  type CefTabInfo,
} from "@/lib/cefBrowserEmbed";
import { isTauriDesktop } from "@/lib/desktopShell";
import { useWorkbenchBrowser } from "../hooks/useWorkbenchBrowser";

type Props = {
  projectId: string;
  conversationSessionId?: string | null;
  active: boolean;
};

export function BrowserPanel({ projectId, conversationSessionId, active }: Props) {
  const t = useT();
  const queryClient = useQueryClient();
  const [designMode, setDesignMode] = useState(false);
  const [designInstruction, setDesignInstruction] = useState("");
  const [sendError, setSendError] = useState<string | null>(null);
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);
  const [cefError, setCefError] = useState<string | null>(null);
  const [cefSurfaceReady, setCefSurfaceReady] = useState(false);
  /** Ref mirror of cefSurfaceReady so ResizeObserver retries show() until created. */
  const cefSurfaceReadyRef = useRef(false);
  cefSurfaceReadyRef.current = cefSurfaceReady;
  const [cefAssetsReady, setCefAssetsReady] = useState(false);
  const [cefTabs, setCefTabs] = useState<CefTabInfo[]>([]);
  const [urlFocused, setUrlFocused] = useState(false);
  const stageRef = useRef<HTMLDivElement>(null);
  const embedHostRef = useRef<HTMLDivElement>(null);
  /** Last CEF URL we pushed into the address bar (avoid fighting the user while typing). */
  const lastSyncedUrlRef = useRef<string>("");
  const urlFocusedRef = useRef(false);
  urlFocusedRef.current = urlFocused;

  useEffect(() => {
    if (!isTauriDesktop() || !active) {
      setCefAssetsReady(false);
      setCefSurfaceReady(false);
      return;
    }
    let cancelled = false;
    void cefBrowserStatus().then((s) => {
      if (!cancelled) setCefAssetsReady(Boolean(s?.ready));
    });
    return () => {
      cancelled = true;
    };
  }, [active]);

  const {
    urlInput,
    setUrlInput,
    navigate,
    refresh,
    lockState,
    unlockForUser,
    screenshot,
    createSession,
    hasMeaningfulUrl,
    setPickMode,
    picked,
    setPicked,
    hoverHit,
    clearHoverHit,
    hitTestAtImagePoint,
    hoverHitTestAtImagePoint,
    setDesignInspect,
    sessionReady,
    deviceWidth,
    deviceHeight,
    status,
  } = useWorkbenchBrowser(projectId, conversationSessionId, active, {
    // Desktop default: native CEF. Assets probe returns ready when frameworks exist
    // and ANYCODE_CEF_EMBED is not explicitly 0.
    preferCef: isTauriDesktop() && cefAssetsReady,
    cefSurfaceReady,
  });

  const useCefEmbed = isTauriDesktop() && (cefAssetsReady || Boolean(status.cefReady));

  useEffect(() => {
    if (designMode) setPickMode(true);
    else setPickMode(false);
  }, [designMode, setPickMode]);

  // CEF: page-side inspect via CDP (React overlay cannot sit above the native view).
  useEffect(() => {
    if (!useCefEmbed || !sessionReady) return;
    void setDesignInspect(designMode && active);
    return () => {
      void setDesignInspect(false);
    };
  }, [useCefEmbed, designMode, active, sessionReady, setDesignInspect]);

  useEffect(() => {
    if (!designMode) {
      setCursor(null);
      clearHoverHit();
    }
  }, [designMode, clearHoverHit]);

  // Embed CEF native view over the panel content rect (not over chrome / design bar).
  // `status.isLoading` is a dep on purpose: while the status query loads, the early
  // return below renders a loading placeholder WITHOUT the host div, so a first-fire
  // of this effect finds `embedHostRef.current === null` and bails. Without the dep
  // the effect never re-fires (deps unchanged) and CEF is never shown — the
  // deterministic "panel open but no live view" failure on cold panel opens.
  const browserStatusLoading = status.isLoading;
  useEffect(() => {
    if (!useCefEmbed || !active) {
      void cefBrowserHide();
      return;
    }
    const el = embedHostRef.current;
    if (!el) return;

    const syncShow = () => {
      const rect = el.getBoundingClientRect();
      if (rect.width < 2 || rect.height < 2) return;
      const payload = {
        x: Math.floor(rect.left),
        y: Math.floor(rect.top),
        width: Math.floor(rect.width),
        height: Math.floor(rect.height),
      };
      // url only used when creating the first CEF browser; later calls only resize.
      void cefBrowserShow(payload, urlInput.trim() || "about:blank")
        .then((s) => {
          setCefError(null);
          setCefSurfaceReady(true);
          if (s.tabs) setCefTabs(s.tabs);
        })
        .catch((e: Error) => {
          setCefError(e.message);
          setCefSurfaceReady(false);
          // Init/show failure → fall back to screencast / headless CDP.
          setCefAssetsReady(false);
        });
    };

    syncShow();
    const ro = new ResizeObserver(() => {
      const rect = el.getBoundingClientRect();
      if (!cefSurfaceReadyRef.current) {
        // Host just got a real size (dock animation, late layout): the initial
        // syncShow may have bailed on a <2px rect — retry creation instead of
        // resize-into-void.
        syncShow();
        return;
      }
      void cefBrowserResize({
        x: Math.floor(rect.left),
        y: Math.floor(rect.top),
        width: Math.floor(rect.width),
        height: Math.floor(rect.height),
      }).catch(() => {
        /* ignore until shown */
      });
    });
    ro.observe(el);
    window.addEventListener("resize", syncShow);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", syncShow);
      setCefSurfaceReady(false);
      void cefBrowserHide();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- urlInput only for first create
  }, [useCefEmbed, active, browserStatusLoading]);

  // Design mode only changes host height — debounce a resize, never recreate CEF.
  useEffect(() => {
    if (!useCefEmbed || !active) return;
    const el = embedHostRef.current;
    if (!el) return;
    let raf = 0;
    raf = window.requestAnimationFrame(() => {
      const rect = el.getBoundingClientRect();
      if (rect.width < 2 || rect.height < 2) return;
      void cefBrowserResize({
        x: Math.floor(rect.left),
        y: Math.floor(rect.top),
        width: Math.floor(rect.width),
        height: Math.floor(rect.height),
      }).catch(() => {
        /* ignore */
      });
    });
    return () => window.cancelAnimationFrame(raf);
  }, [designMode, useCefEmbed, active]);

  // Sync tab strip + address bar from CEF — never while the user is editing the URL.
  useEffect(() => {
    if (!useCefEmbed || !active) return;
    const id = window.setInterval(() => {
      void cefBrowserStatus().then((s) => {
        if (!s) return;
        if (s.tabs) setCefTabs(s.tabs);
        const cefUrl = s.url?.trim();
        if (!cefUrl) return;
        if (urlFocusedRef.current) return;
        if (cefUrl === lastSyncedUrlRef.current) return;
        lastSyncedUrlRef.current = cefUrl;
        setUrlInput(cefUrl);
      });
    }, 700);
    return () => window.clearInterval(id);
  }, [useCefEmbed, active, setUrlInput]);

  // Layout size is CSS device metrics; JPEG may be 2× Retina — keep display at CSS size.
  const displayW = deviceWidth;
  const displayH = deviceHeight;

  const sendDesign = useMutation({
    mutationFn: async (prompt: string) => {
      if (!conversationSessionId) {
        throw new Error(t("conversations.selectSession"));
      }
      return api.sendSessionMessage(conversationSessionId, {
        prompt,
        enqueue: true,
      });
    },
    onSuccess: () => {
      setDesignInstruction("");
      setSendError(null);
      if (conversationSessionId) {
        void queryClient.invalidateQueries({
          queryKey: ["session-transcript", conversationSessionId],
        });
        void queryClient.invalidateQueries({ queryKey: ["session", conversationSessionId] });
      }
    },
    onError: (e: Error) => setSendError(e.message),
  });

  if (status.isLoading) {
    return (
      <p className="text-xs text-secondary text-center py-8 m-0">{t("common.loading")}</p>
    );
  }

  if (!status.chromiumReady && !useCefEmbed) {
    return (
      <div className="px-3 py-4 text-xs text-secondary space-y-2">
        <p className="m-0 font-medium text-on-surface">{t("workbench.browserUnavailableTitle")}</p>
        <p className="m-0">{status.doctorMessage ?? t("settings.browserConnector.notBundled")}</p>
        <ol className="m-0 pl-4 space-y-1">
          <li>{t("workbench.browserSetupStepChrome")}</li>
          <li>{t("workbench.browserSetupStepEnable")}</li>
          <li>{t("workbench.browserSetupStepPanel")}</li>
        </ol>
        <Link to="/settings" search={{ section: "notify" }} className="text-primary">
          {t("workbench.browserSetup")}
        </Link>
      </div>
    );
  }

  const submitDesignChange = () => {
    const instruction = designInstruction.trim();
    if (!instruction || !conversationSessionId) return;
    const prompt = encodeBrowserDesignMessage(
      picked ? { ...picked, url: urlInput } : null,
      instruction,
    );
    sendDesign.mutate(prompt);
  };

  const hoverLabel = hoverHit ? chipLabel(hoverHit) : null;

  return (
    <div className="flex flex-col h-full min-h-0">
      {!status.browserEnabled && (
        <div className="px-2 py-2 text-[10px] text-warn border-b border-outline-variant/60 shrink-0">
          {t("workbench.browserDisabledHint")}{" "}
          <Link to="/settings" search={{ section: "notify" }} className="text-primary">
            {t("workbench.browserSetup")}
          </Link>
        </div>
      )}
      <div className="flex items-center justify-between gap-2 px-2 py-1 border-b border-outline-variant/60 text-[10px] text-secondary shrink-0">
        <span>
          {lockState === "agent"
            ? t("workbench.browserLockAgent")
            : t("workbench.browserLockUser")}
        </span>
        {lockState === "agent" && (
          <button type="button" className="dw-btn-secondary px-2 py-0.5 text-[10px]" onClick={unlockForUser}>
            {t("workbench.browserUnlock")}
          </button>
        )}
      </div>
      {useCefEmbed && (
        <div className="flex items-center gap-1 px-1 py-1 border-b border-outline-variant/60 bg-surface-container-low shrink-0 overflow-x-auto">
          {(cefTabs.length > 0 ? cefTabs : [{ id: -1, url: urlInput, title: urlInput || "New Tab", active: true }]).map(
            (tab) => {
              const isActive = tab.active;
              const label = (tab.title || tab.url || t("workbench.browserNewTab")).slice(0, 28);
              return (
                <span
                  key={tab.id}
                  role="tab"
                  aria-selected={isActive}
                  className={`inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs whitespace-nowrap border cursor-pointer select-none max-w-[10rem] ${
                    isActive
                      ? "bg-surface-container-high text-on-surface border-outline-variant"
                      : "text-secondary border-transparent hover:bg-surface-container-high/60"
                  }`}
                  onClick={() => {
                    if (tab.id < 0) return;
                    void cefBrowserSelectTab(tab.id)
                      .then((s) => {
                        if (s.tabs) setCefTabs(s.tabs);
                        if (s.url) setUrlInput(s.url);
                      })
                      .catch((err: Error) => setCefError(err.message));
                  }}
                >
                  <span className="truncate">{label}</span>
                  {tab.id >= 0 && (
                    <button
                      type="button"
                      className="dw-btn-ghost p-0 text-[10px] leading-none"
                      aria-label={t("workbench.browserCloseTab")}
                      onClick={(e) => {
                        e.stopPropagation();
                        void cefBrowserCloseTab(tab.id)
                          .then((s) => {
                            if (s.tabs) setCefTabs(s.tabs);
                            if (s.url) setUrlInput(s.url);
                          })
                          .catch((err: Error) => setCefError(err.message));
                      }}
                    >
                      <Icon name="close" size={12} />
                    </button>
                  )}
                </span>
              );
            },
          )}
          <button
            type="button"
            className="dw-btn-ghost p-0.5 text-secondary"
            title={t("workbench.browserNewTab")}
            aria-label={t("workbench.browserNewTab")}
            onClick={() => {
              void cefBrowserNewTab("about:blank")
                .then((s) => {
                  if (s.tabs) setCefTabs(s.tabs);
                  setUrlInput(s.url || "about:blank");
                })
                .catch((err: Error) => setCefError(err.message));
            }}
          >
            <Icon name="add" size={16} />
          </button>
        </div>
      )}
      <form
        className="flex items-center gap-1.5 px-2 py-2 border-b border-outline-variant/60 shrink-0"
        onSubmit={(e) => {
          e.preventDefault();
          let target = urlInput.trim();
          if (!target) return;
          if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(target)) {
            target = `https://${target}`;
          }
          lastSyncedUrlRef.current = target;
          setUrlInput(target);
          setUrlFocused(false);
          // CEF embed owns navigation — do not also CDP Page.goto (dual-nav races).
          if (useCefEmbed) {
            void cefBrowserNavigate(target)
              .then((s) => {
                if (s.tabs) setCefTabs(s.tabs);
                if (s.url) {
                  lastSyncedUrlRef.current = s.url;
                  setUrlInput(s.url);
                }
              })
              .catch((err: Error) => setCefError(err.message));
          } else {
            navigate.mutate(target);
          }
        }}
      >
        <input
          type="text"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          className="flex-1 min-w-0 text-xs px-2 py-1 rounded border border-outline-variant bg-surface-container-low"
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          onFocus={() => setUrlFocused(true)}
          onBlur={() => setUrlFocused(false)}
          placeholder="https:// or http://localhost:…"
        />
        <button
          type="button"
          className="dw-btn-secondary p-1.5 shrink-0"
          title={t("workbench.browserRefresh")}
          disabled={navigate.isPending || !hasMeaningfulUrl}
          onClick={() => {
            const target = urlInput.trim();
            if (!target || target === "about:blank") return;
            if (useCefEmbed) {
              void cefBrowserNavigate(target)
                .then((s) => {
                  if (s.tabs) setCefTabs(s.tabs);
                  if (s.url) {
                    lastSyncedUrlRef.current = s.url;
                    setUrlInput(s.url);
                  }
                })
                .catch((err: Error) => setCefError(err.message));
            } else {
              refresh();
            }
          }}
        >
          <Icon name="refresh" size={16} />
        </button>
        <button
          type="button"
          className={`dw-btn-secondary p-1.5 shrink-0 ${
            designMode ? "bg-primary/15 text-primary border-primary/40" : ""
          }`}
          title={t("workbench.browserDesignMode")}
          aria-pressed={designMode}
          aria-label={t("workbench.browserDesignMode")}
          disabled={!useCefEmbed && !screenshot.data?.screenshot.image_base64}
          onClick={() => {
            setDesignMode((v) => {
              const next = !v;
              if (!next) {
                setPickMode(false);
                setPicked(null);
              } else {
                setPickMode(true);
              }
              return next;
            });
          }}
        >
          <Icon name="design_mode" size={16} />
        </button>
        <button
          type="submit"
          className="dw-btn-secondary p-1.5 shrink-0"
          disabled={navigate.isPending}
        >
          <Icon name="arrow_forward" size={16} />
        </button>
      </form>

      {createSession.isError && (
        <div className="px-3 py-3 text-xs text-secondary">
          <p className="m-0 mb-2">{(createSession.error as Error).message}</p>
          <Link to="/settings" search={{ section: "notify" }} className="text-primary">
            {t("workbench.browserSetup")}
          </Link>
        </div>
      )}

      {sendError && (
        <p className="m-0 px-2 py-1 text-[10px] text-error border-b border-outline-variant/60 shrink-0">
          {sendError}
        </p>
      )}

      {cefError && (
        <p className="m-0 px-2 py-1 text-[10px] text-warn border-b border-outline-variant/60 shrink-0">
          CEF: {cefError}
        </p>
      )}

      <div className="conv-browser-viewport flex-1 min-h-0 overflow-hidden bg-surface-container-low p-2 relative">
        {useCefEmbed ? (
          <>
            <div
              ref={embedHostRef}
              className={`conv-browser-cef-host w-full min-h-[240px] rounded border border-outline-variant/40 bg-transparent ${
                designMode ? "h-[calc(100%-4.25rem)]" : "h-full"
              }`}
              aria-label={t("workbench.browserCefEmbed")}
            />
            {designMode && (
              <form
                className="conv-browser-design-bar"
                onSubmit={(e) => {
                  e.preventDefault();
                  submitDesignChange();
                }}
              >
                {picked ? (
                  <span className="conv-browser-chip shrink-0" title={picked.css_selector}>
                    <Icon name="design_mode" size={12} />
                    <span className="truncate max-w-[9rem]">{chipLabel(picked)}</span>
                    <button
                      type="button"
                      className="conv-browser-chip__x"
                      aria-label={t("common.close")}
                      onClick={() => setPicked(null)}
                    >
                      <Icon name="close" size={12} />
                    </button>
                  </span>
                ) : (
                  <Icon name="edit" size={16} className="text-primary shrink-0" />
                )}
                <input
                  type="text"
                  className="conv-browser-design-bar__input"
                  value={designInstruction}
                  onChange={(e) => setDesignInstruction(e.target.value)}
                  placeholder={
                    hoverLabel
                      ? `${t("workbench.browserDesignPlaceholder")} · ${hoverLabel}`
                      : t("workbench.browserDesignPlaceholder")
                  }
                  autoComplete="off"
                  disabled={!conversationSessionId || sendDesign.isPending}
                />
                <button
                  type="submit"
                  className="dw-btn-primary px-2 py-1 text-[11px] shrink-0 rounded-full"
                  disabled={
                    !designInstruction.trim() ||
                    !conversationSessionId ||
                    sendDesign.isPending
                  }
                >
                  {sendDesign.isPending
                    ? t("common.loading")
                    : t("workbench.browserDesignSend")}
                </button>
              </form>
            )}
          </>
        ) : screenshot.data?.screenshot.image_base64 ? (
          <div
            ref={stageRef}
            className="conv-browser-stage relative inline-block"
            style={{ width: displayW, height: displayH }}
          >
            <img
              src={`data:${screenshot.data.screenshot.mime ?? "image/png"};base64,${screenshot.data.screenshot.image_base64}`}
              alt={urlInput}
              width={displayW}
              height={displayH}
              draggable={false}
              className={`conv-browser-frame block max-w-none rounded border border-outline-variant/40 ${
                designMode ? "cursor-none" : ""
              }`}
              style={{ width: displayW, height: displayH }}
              onClick={(e) => {
                if (!designMode) return;
                hitTestAtImagePoint(e.clientX, e.clientY, e.currentTarget);
              }}
              onMouseMove={(e) => {
                if (!designMode) return;
                const stage = stageRef.current;
                if (!stage) return;
                const rect = stage.getBoundingClientRect();
                setCursor({
                  x: e.clientX - rect.left,
                  y: e.clientY - rect.top,
                });
                hoverHitTestAtImagePoint(e.clientX, e.clientY, e.currentTarget);
              }}
              onMouseLeave={() => {
                if (!designMode) return;
                setCursor(null);
                clearHoverHit();
              }}
            />
            {designMode && cursor && (
              <div
                className="conv-browser-inspect-cursor"
                style={{ left: cursor.x, top: cursor.y }}
                aria-hidden
              >
                <span className="conv-browser-inspect-cursor__h" />
                <span className="conv-browser-inspect-cursor__v" />
                <span className="conv-browser-inspect-cursor__dot" />
              </div>
            )}
            {designMode && hoverHit && cursor && (
              <div
                className="conv-browser-inspect-tag"
                style={{
                  left: Math.min(cursor.x + 12, displayW - 8),
                  top: Math.max(0, cursor.y - 28),
                }}
              >
                {chipLabel(hoverHit)}
              </div>
            )}
            {designMode && (
              <form
                className="conv-browser-design-bar"
                onSubmit={(e) => {
                  e.preventDefault();
                  submitDesignChange();
                }}
              >
                {picked ? (
                  <span className="conv-browser-chip shrink-0" title={picked.css_selector}>
                    <Icon name="design_mode" size={12} />
                    <span className="truncate max-w-[9rem]">{chipLabel(picked)}</span>
                    <button
                      type="button"
                      className="conv-browser-chip__x"
                      aria-label={t("common.close")}
                      onClick={() => setPicked(null)}
                    >
                      <Icon name="close" size={12} />
                    </button>
                  </span>
                ) : (
                  <Icon name="edit" size={16} className="text-primary shrink-0" />
                )}
                <input
                  type="text"
                  className="conv-browser-design-bar__input"
                  value={designInstruction}
                  onChange={(e) => setDesignInstruction(e.target.value)}
                  placeholder={t("workbench.browserDesignPlaceholder")}
                  autoComplete="off"
                  disabled={!conversationSessionId || sendDesign.isPending}
                />
                <button
                  type="submit"
                  className="dw-btn-primary px-2 py-1 text-[11px] shrink-0 rounded-full"
                  disabled={
                    !designInstruction.trim() ||
                    !conversationSessionId ||
                    sendDesign.isPending
                  }
                >
                  {sendDesign.isPending
                    ? t("common.loading")
                    : t("workbench.browserDesignSend")}
                </button>
              </form>
            )}
          </div>
        ) : (
          <div className="text-xs text-secondary text-center py-8 m-0 space-y-2">
            <p className="m-0">
              {createSession.isPending
                ? t("common.loading")
                : hasMeaningfulUrl
                  ? t("workbench.browserNavigatedWaiting").replace("{url}", urlInput)
                  : t("workbench.browserEmpty")}
            </p>
            {!createSession.isPending && !createSession.isError && !hasMeaningfulUrl && (
              <p className="m-0 text-[10px] opacity-80">{t("workbench.browserAgentHint")}</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
