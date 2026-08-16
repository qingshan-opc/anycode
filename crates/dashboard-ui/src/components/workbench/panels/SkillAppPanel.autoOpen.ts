import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { WorkbenchTab } from "@/api/types/workbench";
import { skillAppStore } from "@/lib/skillAppStore";
import { workbenchSidebarStore } from "../hooks/useWorkbenchSidebarState";

export type SkillAppAutoOpenCtx = {
  sessionId: string | null;
  projectId: string | null;
  openTab: (tab: WorkbenchTab, opts?: { focus?: unknown }) => void;
};

/**
 * Poll pending SkillAppPresent records and open the skillApp panel once per
 * wait_brief present_id. Non-waiting presents (Skill tool / Push) are ignored —
 * the HITL studio is host-driven and dismissed after brief.submit.
 */
export function useSkillAppAutoOpen({ sessionId, projectId }: SkillAppAutoOpenCtx): void {
  const seen = useRef(new Set<string>());

  useEffect(() => {
    seen.current.clear();
  }, [sessionId]);

  const pending = useQuery({
    queryKey: ["skill-app-pending", sessionId],
    queryFn: () => api.pendingSkillApps(sessionId ?? undefined),
    enabled: Boolean(sessionId),
    refetchInterval: sessionId ? 400 : false,
  });

  useEffect(() => {
    const rows = pending.data?.pending ?? [];
    for (const row of rows) {
      // Only open a new wait_brief HITL cycle. Push / non-wait presents must not
      // steal focus after the user already dismissed the studio.
      if (!row.wait_brief) continue;
      if (seen.current.has(row.present_id)) continue;
      seen.current.add(row.present_id);

      let slot = row.slot || "conversation";
      if (slot === "project" || slot === "dock") slot = "conversation";

      skillAppStore.setFocus({
        skillId: row.skill_id,
        presentId: row.present_id,
        slot,
        waitBrief: true,
        push: row.push,
      });
      if (projectId) {
        void api.bindProjectSkillApp(projectId, {
          skill_id: row.skill_id,
          slot: row.slot === "project" ? "project" : slot,
          enabled: true,
        });
      }

      workbenchSidebarStore.moveToConversationTab("skillApp");
      workbenchSidebarStore.openTab("skillApp");
    }
  }, [pending.data, projectId]);
}
