import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { workbenchSidebarStore } from "@/components/workbench/hooks/useWorkbenchSidebarState";
import { useT } from "@/i18n/context";
import { attachSkillAppBridge, pushToSkillApp } from "@/lib/skillAppBridge";
import {
  buildSkillAppContinuationPrompt,
  originalPromptFromPush,
} from "@/lib/skillAppPrompt";
import { skillAppStore } from "@/lib/skillAppStore";

type Props = {
  projectId: string | null;
  sessionId: string;
  active: boolean;
};

/** End this present cycle: clear focus (unmount iframe) and remove the main-area tab. */
function dismissSkillAppUi() {
  skillAppStore.dismiss();
  workbenchSidebarStore.closeConversationTab("skillApp");
}

export function SkillAppPanel({ projectId, sessionId, active }: Props) {
  const t = useT();
  const qc = useQueryClient();
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const focus = useSyncExternalStore(skillAppStore.subscribe, skillAppStore.getFocus, () => null);
  const [localError, setLocalError] = useState<string | null>(null);

  const skillId = focus?.skillId ?? null;

  const stateQ = useQuery({
    queryKey: ["skill-app-state", projectId, skillId],
    queryFn: () => api.skillAppState(projectId!, skillId!),
    enabled: Boolean(projectId && skillId),
  });

  const putState = useMutation({
    mutationFn: (body: { state?: unknown; brief?: unknown }) =>
      api.putSkillAppState(projectId!, skillId!, body),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["skill-app-state", projectId, skillId] });
    },
  });

  const submitBrief = useMutation({
    mutationFn: (brief: unknown) =>
      api.submitSkillAppBrief(projectId!, {
        skill_id: skillId!,
        brief,
        present_id: skillAppStore.getFocus()?.presentId,
      }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["skill-app-state", projectId, skillId] });
    },
  });

  const sendContinuation = useMutation({
    mutationFn: async (payload: { text?: string; brief?: unknown }) => {
      const cur = skillAppStore.getFocus();
      const id = cur?.skillId ?? skillId ?? "skill";
      const text =
        payload.text?.trim() ||
        buildSkillAppContinuationPrompt({
          skillId: id,
          brief: payload.brief,
          originalPrompt: originalPromptFromPush(cur?.push),
        });
      await api.sendSessionMessage(sessionId, { prompt: text, enqueue: false });
    },
  });

  const src = skillId ? api.skillAppAssetUrl(skillId, "index.html") : null;

  useEffect(() => {
    if (!iframeRef.current || !skillId || !projectId) return;
    const detach = attachSkillAppBridge(iframeRef.current, {
      onStateGet: async () => {
        const data = await api.skillAppState(projectId, skillId);
        const focusNow = skillAppStore.getFocus();
        return {
          state: data.state,
          brief: data.brief,
          present_id: focusNow?.presentId ?? null,
          wait_brief: focusNow?.waitBrief === true,
        };
      },
      onStateSet: async (patch) => {
        const next =
          patch && typeof patch === "object" && "state" in (patch as object)
            ? (patch as { state: unknown }).state
            : patch;
        await putState.mutateAsync({ state: next });
        return { ok: true };
      },
      onBriefSubmit: async (brief) => {
        const cur = skillAppStore.getFocus();
        const waiting = cur?.waitBrief === true;
        const result = await submitBrief.mutateAsync(brief);
        const resumed = result.resumed === true || waiting;
        if (!resumed) {
          await sendContinuation.mutateAsync({ brief });
        }
        dismissSkillAppUi();
        return { ok: true, resumed };
      },
      onAgentPrompt: async (payload) => {
        const cur = skillAppStore.getFocus();
        if (cur?.waitBrief) {
          // Waiting turn already owns the prompt; do not send a duplicate.
          dismissSkillAppUi();
          return { ok: true, resumed: true };
        }
        await sendContinuation.mutateAsync(payload);
        dismissSkillAppUi();
        return { ok: true };
      },
      onBackToChat: () => {
        dismissSkillAppUi();
        return { ok: true };
      },
    });
    return detach;
  }, [skillId, projectId, putState, submitBrief, sendContinuation]);

  useEffect(() => {
    if (focus?.waitBrief) {
      pushToSkillApp(iframeRef.current, { wait_brief: true });
    }
  }, [focus?.waitBrief, focus?.presentId]);

  const onLoad = useCallback(() => {
    if (focus?.push != null) {
      pushToSkillApp(iframeRef.current, focus.push);
    }
    if (focus?.waitBrief) {
      pushToSkillApp(iframeRef.current, { wait_brief: true });
    }
  }, [focus?.push, focus?.waitBrief]);

  if (!projectId) {
    return (
      <p className="text-sm text-secondary px-4 py-6 m-0 text-center">{t("workbench.noProject")}</p>
    );
  }

  if (!skillId || !src) {
    return (
      <EmptyState
        title={t("workbench.skillAppEmpty")}
        description={t("workbench.skillAppEmptyDesc")}
        icon="dashboard_customize"
      />
    );
  }

  if (!active) {
    // Keep iframe mounted for persistence when panel is in DOM but inactive —
    // parent still mounts us; we only hide visually.
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="px-3 py-2 text-xs text-secondary border-b border-outline-variant flex items-center justify-between gap-2 shrink-0">
        <span className="truncate font-medium text-on-surface">{skillId}</span>
        <div className="flex items-center gap-2 shrink-0">
          {focus?.waitBrief ? (
            <span className="text-primary">{t("workbench.skillAppWaitingBrief")}</span>
          ) : null}
          {stateQ.data?.brief ? (
            <span className="text-secondary">{t("workbench.skillAppBriefLocked")}</span>
          ) : null}
          <button
            type="button"
            className="dw-btn-ghost text-xs px-2 py-1 inline-flex items-center gap-1"
            onClick={dismissSkillAppUi}
          >
            <Icon name="chevron_left" size={14} />
            {t("workbench.skillAppBackToChat")}
          </button>
        </div>
      </div>
      {localError ? (
        <p className="text-xs text-error px-3 py-2 m-0">{localError}</p>
      ) : null}
      <iframe
        ref={iframeRef}
        title={`skill-app-${skillId}`}
        src={src}
        className="flex-1 w-full border-0 bg-surface-container-lowest min-h-[16rem]"
        sandbox="allow-scripts"
        onLoad={onLoad}
        onError={() => setLocalError(t("workbench.skillAppLoadError"))}
      />
    </div>
  );
}
