import { Suspense, useCallback, useEffect, useRef } from "react";
import { Link } from "@tanstack/react-router";
import type { WorkbenchTab } from "@/api/types/workbench";
import { ConversationThread } from "@/components/ConversationThread";
import { ProjectGroupedSessionList } from "@/components/session/ProjectGroupedSessionList";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { ConversationWorkbenchHeaderIcons } from "@/components/workbench/ConversationWorkbenchHeaderIcons";
import { WorkbenchPanel } from "@/components/workbench/WorkbenchPanel";
import { useWorkbenchSidebarState } from "@/components/workbench/hooks/useWorkbenchSidebarState";
import { useWorkbenchAutoOpen } from "@/components/workbench/hooks/useWorkbenchAutoOpen";
import { useWorkbenchBadges } from "@/components/workbench/hooks/useWorkbenchBadges";
import { workbenchPanelById } from "@/components/workbench/registry";
import { useConversationShell } from "@/context/ConversationShellContext";
import { useT } from "@/i18n/context";
import { isBrowserToolBlock } from "@/lib/browserToolDetect";

export function ConversationWorkspace() {
  const t = useT();
  const {
    sessionsDrawerOpen,
    setSessionsDrawerOpen,
    setWorkbenchDrawerOpen,
    selectedTool,
    setSelectedTool,
    active,
    rows,
    sidebarFilteredRows,
    listSearch,
    displaySessionId,
    selected,
    selectSession,
    pendingCounts,
    sessionsLoading,
    sessionsError,
    pendingCountsLoading,
    sseLive,
    liveBlocks,
    liveEvents,
    chatStreamLive,
    sessionLive,
    questionsRespondAllowed,
    approvalsRespondAllowed,
    sseStatus,
    isOptimisticStreaming,
    markSessionStreaming,
    clearOptimisticStreaming,
    projectOptions,
    prefetchSession,
    startSessionForProject,
    onRenameSession,
    onRenameProject,
    onRemoveProject,
    optimisticStreamingSessionId,
  } = useConversationShell();

  const {
    expanded: workbenchExpanded,
    activeTab: workbenchTab,
    panelWidth,
    selectTab,
    setExpanded: setWorkbenchExpanded,
    setPanelWidth,
    openTab,
    markSeen,
  } = useWorkbenchSidebarState();

  const resizeRef = useRef<{ startX: number; startW: number } | null>(null);

  useEffect(() => {
    setWorkbenchExpanded(false);
  }, [displaySessionId, setWorkbenchExpanded]);

  useEffect(() => {
    setWorkbenchDrawerOpen(workbenchExpanded);
  }, [workbenchExpanded, setWorkbenchDrawerOpen]);

  useEffect(() => {
    if (!workbenchExpanded) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      setWorkbenchExpanded(false);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [workbenchExpanded, setWorkbenchExpanded]);

  const projectId = selected?.project_id ?? null;

  // Panel auto-open behaviors (browser CEF attach timing, plan review) live in
  // each panel's `*.autoOpen.ts`, composed here in a fixed order.
  useWorkbenchAutoOpen({
    sessionId: displaySessionId,
    projectId,
    liveBlocks,
    streamLive: chatStreamLive || sseLive,
    openTab,
  });

  // Unread badges on the tab icons (artifacts: new deliverables since the
  // user last opened the panel).
  const workbenchBadges = useWorkbenchBadges(displaySessionId);

  // Opening the Artifacts panel marks its deliverables as seen (badge clears).
  useEffect(() => {
    if (workbenchExpanded && workbenchTab === "artifacts") {
      markSeen("artifacts");
    }
  }, [workbenchExpanded, workbenchTab, markSeen]);

  const onResizeStart = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      resizeRef.current = { startX: e.clientX, startW: panelWidth };
      const onMove = (ev: PointerEvent) => {
        if (!resizeRef.current) return;
        const delta = resizeRef.current.startX - ev.clientX;
        setPanelWidth(resizeRef.current.startW + delta);
      };
      const onUp = () => {
        resizeRef.current = null;
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [panelWidth, setPanelWidth],
  );

  const renderWorkbenchPanel = () => {
    if (!displaySessionId) {
      return (
        <p className="text-sm text-secondary px-4 py-6 m-0 text-center">
          {t("conversations.selectSession")}
        </p>
      );
    }
    const def = workbenchPanelById(workbenchTab);
    if (def.needsProject && !projectId) {
      return (
        <p className="text-sm text-secondary px-4 py-6 m-0 text-center">
          {t("workbench.noProject")}
        </p>
      );
    }
    const PanelComponent = def.component;
    return (
      <Suspense
        fallback={
          <p className="text-xs text-secondary text-center py-8 m-0">{t("common.loading")}</p>
        }
      >
        <PanelComponent
          projectId={projectId}
          sessionId={displaySessionId}
          active={workbenchExpanded}
          isRunning={selected?.status === "running"}
          collapse={() => setWorkbenchExpanded(false)}
        />
      </Suspense>
    );
  };

  if (sessionsError) {
    return (
      <div className="dw-alert-error">
        <p className="m-0 font-medium">{t("common.error")}</p>
        <p className="m-0 mt-1 text-sm">{sessionsError.message}</p>
      </div>
    );
  }

  if (sessionsLoading) {
    return <p className="text-sm text-secondary p-4">{t("common.loading")}</p>;
  }

  if (active === "needs_approval" && pendingCountsLoading && rows.length === 0) {
    return <p className="text-sm text-secondary p-4">{t("common.loading")}</p>;
  }

  if (rows.length === 0 && active === "all" && sidebarFilteredRows.length === 0) {
    return (
      <div className="p-6 border border-outline-variant rounded-lg bg-surface-container-lowest m-4">
        <EmptyState
          title={t("conversations.emptyTitle")}
          description={t("conversations.emptyDesc")}
          icon="forum"
        />
        <div className="text-center mt-4">
          <Link to="/" className="dw-btn-primary no-underline inline-flex">
            {t("conversations.newSession")}
          </Link>
        </div>
      </div>
    );
  }

  if (rows.length === 0 && active !== "all" && !selected) {
    return (
      <EmptyState
        title={
          active === "needs_approval"
            ? t("conversations.emptyNeedsApproval")
            : t("conversations.emptyFilter")
        }
        description={active === "needs_approval" ? t("conversations.emptyNeedsApprovalDesc") : undefined}
        icon="forum"
      />
    );
  }

  if (!selected && rows.length === 0) {
    return null;
  }

  const onSelectWorkbenchTab = (tab: WorkbenchTab) => {
    selectTab(tab);
  };

  return (
    <>
      <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
        <div className="lg:hidden flex items-center justify-between gap-2 px-3 py-2 border-b border-outline-variant bg-surface-container-low shrink-0">
          <button
            type="button"
            className="dw-btn-secondary text-xs"
            onClick={() => setSessionsDrawerOpen(true)}
          >
            <Icon name="forum" size={16} />
            {t("conversations.sessionList")}
          </button>
        </div>

        <div
          className={`flex flex-1 min-h-0 min-w-0 overflow-hidden${
            workbenchExpanded ? " conv-session-split--workbench" : ""
          }`}
        >
          <div
            className={`flex flex-col flex-1 min-h-0 min-w-0 overflow-hidden${
              workbenchExpanded ? " conv-thread--workbench-open" : ""
            }`}
          >
            <ConversationThread
              session={selected}
              onFollowUpStarted={selectSession}
              showHeader={true}
              sseLive={sseLive}
              liveBlocks={liveBlocks}
              liveEvents={liveEvents}
              chatStreamLive={chatStreamLive}
              sessionLive={sessionLive}
              questionsRespondAllowed={questionsRespondAllowed}
              approvalsRespondAllowed={approvalsRespondAllowed}
              pendingApprovalCount={
                selected ? (pendingCounts.get(selected.id) ?? 0) : 0
              }
              sseStatus={sseStatus}
              isOptimisticStreaming={isOptimisticStreaming}
              markSessionStreaming={markSessionStreaming}
              clearOptimisticStreaming={clearOptimisticStreaming}
              selectedToolId={selectedTool?.id ?? null}
              onSelectTool={(tool) => {
                setSelectedTool(tool);
                if (isBrowserToolBlock(tool)) {
                  openTab("browser");
                }
              }}
              onRenameSession={onRenameSession}
              headerEnd={
                <ConversationWorkbenchHeaderIcons
                  activeTab={workbenchTab}
                  expanded={workbenchExpanded}
                  onSelectTab={onSelectWorkbenchTab}
                  disabled={!displaySessionId}
                  badges={workbenchBadges}
                />
              }
            />
          </div>

          {workbenchExpanded ? (
            <div
              className="conv-workbench-dock"
              style={{ flex: `1 1 ${panelWidth}px`, minWidth: panelWidth }}
            >
              <WorkbenchPanel
                activeTab={workbenchTab}
                width={panelWidth}
                onResizeStart={onResizeStart}
                onCollapse={() => setWorkbenchExpanded(false)}
              >
                {renderWorkbenchPanel()}
              </WorkbenchPanel>
            </div>
          ) : null}
        </div>
      </div>

      {sessionsDrawerOpen && (
        <>
          <button
            type="button"
            className="fixed inset-0 z-40 bg-black/30 lg:hidden"
            aria-label={t("common.back")}
            onClick={() => setSessionsDrawerOpen(false)}
          />
          <div className="fixed inset-y-0 left-0 z-50 w-[min(100%,20rem)] lg:hidden shadow-xl">
            <div className="h-full border-r border-outline-variant bg-surface-container-lowest flex flex-col">
              <div className="px-3 py-2 text-xs font-semibold uppercase tracking-wide text-secondary border-b border-outline-variant bg-surface-container-low shrink-0 flex items-center justify-between">
                <span>{t("conversations.sessionList")}</span>
                <button
                  type="button"
                  className="dw-btn-ghost p-1"
                  onClick={() => setSessionsDrawerOpen(false)}
                >
                  <Icon name="close" size={18} />
                </button>
              </div>
              <div className="flex-1 min-h-0 overflow-y-auto">
                <ProjectGroupedSessionList
                  projectOptions={projectOptions}
                  sessions={sidebarFilteredRows}
                  selectedId={displaySessionId}
                  onSelect={(id) => {
                    selectSession(id);
                    setSessionsDrawerOpen(false);
                  }}
                  pendingCounts={pendingCounts}
                  onPrefetch={prefetchSession}
                  hideEmptyProjects={listSearch.trim().length > 0}
                  onNewSession={startSessionForProject}
                  onRenameSession={onRenameSession}
                  onRenameProject={onRenameProject}
                  onRemoveProject={onRemoveProject}
                  optimisticStreamingSessionId={optimisticStreamingSessionId}
                />
              </div>
            </div>
          </div>
        </>
      )}
    </>
  );
}
