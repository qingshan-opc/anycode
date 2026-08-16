import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";
import { skillAppStore } from "@/lib/skillAppStore";
import { workbenchSidebarStore } from "@/components/workbench/hooks/useWorkbenchSidebarState";

type Props = {
  projectId: string;
};

/** Pins under a project group — open Skill Apps without relying on the dock. */
export function ProjectSkillAppPins({ projectId }: Props) {
  const t = useT();
  const apps = useQuery({
    queryKey: ["project-skill-apps", projectId],
    queryFn: () => api.projectSkillApps(projectId),
    staleTime: 15_000,
  });
  const pins = (apps.data?.apps ?? []).filter(
    (a) => a.enabled && (a.slot === "project" || a.has_ui),
  );
  if (pins.length === 0) return null;

  return (
    <div className="dw-project-skill-pins px-2 pb-1 flex flex-wrap gap-1" aria-label={t("workbench.skillAppPin")}>
      {pins.map((app) => (
        <button
          key={app.skill_id}
          type="button"
          className="dw-btn-ghost text-[11px] px-1.5 py-0.5 inline-flex items-center gap-1 rounded border border-outline-variant"
          title={app.title || app.skill_id}
          onClick={(e) => {
            e.stopPropagation();
            skillAppStore.setFocus({
              skillId: app.skill_id,
              slot: "conversation",
            });
            // Prefer conversation slot so left sidebar stays visible.
            workbenchSidebarStore.moveToConversationTab("skillApp");
            workbenchSidebarStore.openTab("skillApp");
          }}
        >
          <Icon name="dashboard_customize" size={12} />
          <span className="truncate max-w-[7rem]">{app.title || app.skill_id}</span>
        </button>
      ))}
    </div>
  );
}
