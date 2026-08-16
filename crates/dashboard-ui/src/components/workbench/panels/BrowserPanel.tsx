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
  type CefEmbedStatus,
  type CefTabInfo,
} from "@/lib/cefBrowserEmbed";
import { isTauriDesktop } from "@/lib/desktopShell";
import { useWorkbenchBrowser } from "../hooks/useWorkbenchBrowser";
import { workbenchSidebarStore } from "../hooks/useWorkbenchSidebarState";
import { AgentControlIndicator } from "./AgentControlIndicator";

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
  /**
   * After the user closes the last tab, ResizeObserver / layout sync must not
   * call show_in_parent (that recreates about:blank). Cleared on intentional
   * navigate or "+".
   */
  const suppressAutoCreateRef = useRef(false);

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
    // and ANYCODE_CEF_EMBED is not explicitly 0. Prefer JPEG screencast fallback:
    // launch with ANYCODE_CEF_EMBED=0 (no Workbench settings toggle yet).
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
      // User closed every tab — never recreate via layout churn.
      if (suppressAutoCreateRef.current) {
        void cefBrowserResize(payload).catch(() => {
          /* ignore while empty */
        });
        return;
      }
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
      if (!cefSurfaceReadyRef.current || suppressAutoCreateRef.current) {
        // Host just got a real size (dock animation, late layout): the initial
        // syncShow may have bailed on a <2px rect — retry creation instead of
        // resize-into-void. After last-tab close, syncShow is a no-create resize.
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

  // Navigate the CEF surface; after a last-tab close there is no browser to
  // navigate, so recreate one via show (show_in_parent creates when tabs==0).
  const navigateCef = (target: string) => {
    const applyStatus = (s: CefEmbedStatus) => {
      if (s.tabs) setCefTabs(s.tabs);
      if (s.url) {
        lastSyncedUrlRef.current = s.url;
        setUrlInput(s.url);
      }
    };
    suppressAutoCreateRef.current = false;
    if (cefTabs.length === 0) {
      const el = embedHostRef.current;
      const rect = el?.getBoundingClientRect();
      if (!rect || rect.width < 2 || rect.height < 2) {
        setCefError("browser surface not ready");
        return;
      }
      void cefBrowserShow(
        {
          x: Math.floor(rect.left),
          y: Math.floor(rect.top),
          width: Math.floor(rect.width),
          height: Math.floor(rect.height),
        },
        target,
      )
        .then((s) => {
          setCefSurfaceReady(true);
          applyStatus(s);
        })
        .catch((err: Error) => setCefError(err.message));
      return;
    }
    void cefBrowserNavigate(target).then(applyStatus).catch((err: Error) => setCefError(err.message));
  };

  const applyTabsFromStatus = (s: CefEmbedStatus) => {
    const tabs = s.tabs ?? [];
    setCefTabs(tabs);
    if (tabs.length === 0) {
      suppressAutoCreateRef.current = true;
      setCefSurfaceReady(false);
      return;
    }
    if (s.url) {
      lastSyncedUrlRef.current = s.url;
      setUrlInput(s.url);
    }
  };

  // Design mode and the agent-control pill only change host height — debounce a
  // resize, never recreate CEF.
  const agentLocked = lockState === "agent";
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
  }, [designMode, agentLocked, useCefEmbed, active]);

  // Sync tab strip + address bar from CEF — never while the user is editing the URL.
  useEffect(() => {
    if (!useCefEmbed || !active) return;
    const id = window.setInterval(() => {
      void cefBrowserStatus().then((s) => {
        if (!s) return;
        if (s.tabs) {
          setCefTabs(s.tabs);
          // Do not flip suppressAutoCreate here — a cold poll before the first
          // show completes would permanently block browser creation.
          if (s.tabs.length === 0 && suppressAutoCreateRef.current) {
            setCefSurfaceReady(false);
          }
        }
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
        <div className="px-2 py-2 text-xs text-warn border-b border-outline-variant/60 shrink-0">
          {t("workbench.browserDisabledHint")}{" "}
          <Link to="/settings" search={{ section: "notify" }} className="text-primary">
            {t("workbench.browserSetup")}
          </Link>
        </div>
      )}
      {useCefEmbed && (
        <div className="flex items-center gap-1 px-1 py-1 border-b border-outline-variant/60 bg-surface-container-low shrink-0 overflow-x-auto">
          {cefTabs.map((tab) => {
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
                  void cefBrowserSelectTab(tab.id)
                    .then(applyTabsFromStatus)
                    .catch((err: Error) => setCefError(err.message));
                }}
              >
                <span className="truncate min-w-0">{label}</span>
                <button
                  type="button"
                  className="dw-btn-ghost p-0 leading-none shrink-0"
                  aria-label={t("workbench.browserCloseTab")}
                  onClick={(e) => {
                    e.stopPropagation();
                    void cefBrowserCloseTab(tab.id)
                      .then((s) => {
                        applyTabsFromStatus(s);
                        // Last tab closed by the user: collapse the panel
                        // surface (dock or main-area tab) instead of leaving an
                        // empty browser area on screen. Poll-driven tab
                        // transitions must NOT collapse — a cold poll sees
                        // tabs==0 before the first show completes.
                        if ((s.tabs ?? []).length === 0) {
                          workbenchSidebarStore.collapseTab("browser");
                        }
                      })
                      .catch((err: Error) => setCefError(err.message));
                  }}
                >
                  <Icon name="close" size={12} />
                </button>
              </span>
            );
          })}
          <button
            type="button"
            className="dw-btn-ghost p-0.5 text-secondary shrink-0"
            title={t("workbench.browserNewTab")}
            aria-label={t("workbench.browserNewTab")}
            onClick={() => {
              suppressAutoCreateRef.current = false;
              // Zero tabs: new_tab needs a live container — recreate via show first.
              if (cefTabs.length === 0) {
                const el = embedHostRef.current;
                const rect = el?.getBoundingClientRect();
                if (!rect || rect.width < 2 || rect.height < 2) {
                  setCefError("browser surface not ready");
                  return;
                }
                void cefBrowserShow(
                  {
                    x: Math.floor(rect.left),
                    y: Math.floor(rect.top),
                    width: Math.floor(rect.width),
                    height: Math.floor(rect.height),
                  },
                  "about:blank",
                )
                  .then((s) => {
                    setCefSurfaceReady(true);
                    applyTabsFromStatus(s);
                    setUrlInput(s.url || "about:blank");
                  })
                  .catch((err: Error) => setCefError(err.message));
                return;
              }
              void cefBrowserNewTab("about:blank")
                .then((s) => {
                  setCefSurfaceReady(true);
                  applyTabsFromStatus(s);
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
            navigateCef(target);
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
              navigateCef(target);
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
        <p className="m-0 px-2 py-1 text-xs text-error border-b border-outline-variant/60 shrink-0">
          {sendError}
        </p>
      )}

      {cefError && (
        <p className="m-0 px-2 py-1 text-xs text-warn border-b border-outline-variant/60 shrink-0">
          CEF: {cefError}
        </p>
      )}

      <div className="conv-browser-viewport flex-1 min-h-0 overflow-hidden bg-surface-container-low p-2 relative">
        {useCefEmbed ? (
          <>
            <div
              ref={embedHostRef}
              className={`conv-browser-cef-host w-full min-h-[240px] rounded border border-outline-variant/40 bg-transparent ${
                designMode
                  ? "h-[calc(100%-4.25rem)]"
                  : agentLocked
                    ? "h-[calc(100%-2.75rem)]"
                    : "h-full"
              }`}
              aria-label={t("workbench.browserCefEmbed")}
            />
            {cefTabs.length === 0 && (
              <div className="absolute inset-0 flex items-center justify-center pointer-events-none px-4">
                <p className="m-0 text-xs text-secondary text-center">
                  {t("workbench.browserEmpty")}
                </p>
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
                  className="dw-btn-primary px-2 py-1 text-xs shrink-0 rounded-full"
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
                  className="dw-btn-primary px-2 py-1 text-xs shrink-0 rounded-full"
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
              <p className="m-0 text-xs opacity-80">{t("workbench.browserAgentHint")}</p>
            )}
          </div>
        )}
        {agentLocked && <AgentControlIndicator onTakeControl={unlockForUser} />}
      </div>
    </div>
  );
}
