import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import { api } from "@/api/client";
import { HomeHeroComposer } from "@/components/HomeHeroComposer";
import { NewProjectDialog } from "@/components/NewProjectDialog";
import { usePendingApprovalCounts } from "@/components/SecurityApprovalInbox";
import { useConversationShell } from "@/context/ConversationShellContext";
import { useSseStatus } from "@/context/SseContext";
import { useT } from "@/i18n/context";
import { apiConnectionMessage } from "@/lib/apiConnectionMessage";
import { consumeComposerSeed } from "@/lib/composerSeed";
import { isTauriDesktop } from "@/lib/desktopShell";
import type { EmbeddedPageProps } from "@/lib/pageProps";

export function HomePage(_props: EmbeddedPageProps = {}) {
  const t = useT();
  const sseStatus = useSseStatus();
  const [newProjectOpen, setNewProjectOpen] = useState(false);
  const [prompt, setPrompt] = useState("");
  const homeSearch = useSearch({ from: "/_shell/", shouldThrow: false }) as
    | { project?: string }
    | undefined;
  const { projectOptions, projectsError, projectId, goHome, beginPendingSession, markSessionStreaming } =
    useConversationShell();
  const health = useQuery({
    queryKey: ["health"],
    queryFn: api.health,
    retry: isTauriDesktop() ? 20 : 2,
    retryDelay: (attempt) => Math.min(500 * 2 ** attempt, 4_000),
  });
  const overview = useQuery({ queryKey: ["overview"], queryFn: api.overview });
  // 首次上手：模型（BYOK）是唯一阻塞步骤。未配置模型且从未完成过引导 →
  // 直接进 /setup；引导过但仍缺模型（用户跳过）→ 只显示 CTA 横幅。
  const setupStatus = useQuery({
    queryKey: ["setup", "status"],
    queryFn: api.setupStatus,
    staleTime: 60_000,
  });
  const setup = setupStatus.data?.setup;
  const modelMissing = setup
    ? setup.steps.some((s) => s.id === "llm" && !s.complete)
    : false;
  const { pendingTotal } = usePendingApprovalCounts();

  useEffect(() => {
    const seed = consumeComposerSeed();
    if (seed) setPrompt(seed);
  }, []);

  if (health.isLoading || (health.isError && health.isFetching)) {
    const msg = apiConnectionMessage(t, "loading");
    return (
      <div className="flex items-center justify-center min-h-[40vh] text-sm text-secondary">
        {msg.text}
      </div>
    );
  }

  if (health.isError) {
    const msg = apiConnectionMessage(t, "error");
    return (
      <div className="dw-alert-error">
        <p className="text-sm m-0">{msg.text}</p>
        {msg.showLoopbackHint ? (
          <p className="text-sm m-0 mt-2 text-secondary">
            For shipping builds open the <strong>anyCode.app</strong> Desktop
            window. Developer-only:{" "}
            <code className="font-code">anycode-dashboard-serve</code> on{" "}
            <code className="font-code">127.0.0.1:43180</code>.
          </p>
        ) : null}
      </div>
    );
  }

  const ov = overview.data?.overview;
  const effectiveProjectId =
    projectId || homeSearch?.project || projectOptions[0]?.id || "";

  return (
    <>
      <NewProjectDialog
        open={newProjectOpen}
        onClose={() => setNewProjectOpen(false)}
        navigateOnSuccess={false}
        onCreated={(project) => {
          goHome(project.id);
        }}
      />

      <div className="dw-home-stage">
        <section className="dw-home-hero">
          <div className="hero-glow" aria-hidden />
          <div className="dw-home-hero__intro">
            <h1 className="dw-hero__title m-0">
              {t("home.hero.titleLead")}
              <span className="accent-text">{t("home.hero.titleAccent")}</span>
              {t("home.hero.titleRest")}
            </h1>
            <p className="dw-home-hero__subtitle">{t("home.hero.subtitle")}</p>
          </div>
          {projectsError ? (
            <div className="dw-alert-error mb-4">
              {/\b401\b/.test(projectsError.message)
                ? t("projects.authError")
                : projectsError.message || t("projects.loadError")}
            </div>
          ) : null}
          {modelMissing ? (
            <div className="dw-alert-error mb-4 flex items-center justify-between gap-3">
              <span className="text-sm">{t("setup.welcome.pointKey")}</span>
              <Link to="/cloud-login" className="dw-btn-primary shrink-0">
                {t("service.cloud.gateCta")}
              </Link>
            </div>
          ) : null}
          <HomeHeroComposer
            sseStatus={sseStatus}
            projectOptions={projectOptions.map((p) => ({ id: p.id, name: p.name }))}
            projectId={effectiveProjectId}
            onProjectChange={goHome}
            initialProjectId={homeSearch?.project}
            blockedCount={ov?.sessions_blocked ?? 0}
            pendingCount={pendingTotal}
            budgetExceededCount={ov?.sessions_budget_exceeded ?? 0}
            prompt={prompt}
            onPromptChange={setPrompt}
            onSelectDirectory={() => setNewProjectOpen(true)}
            onSessionStarted={({ session, projectId: pid, projectName }) => {
              beginPendingSession(session, { id: pid, name: projectName });
              markSessionStreaming(session.id);
            }}
          />
        </section>
      </div>
    </>
  );
}
