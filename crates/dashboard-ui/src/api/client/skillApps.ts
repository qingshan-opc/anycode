import { del, get, post, put } from "../http";
import type {
  PendingSkillAppPresent,
  SkillAppBinding,
  SkillAppCatalogItem,
  VisualBrief,
} from "../types/skillApps";

export const skillAppsClient = {
  skillAppsCatalog: () => get<{ apps: SkillAppCatalogItem[] }>("/api/skill-apps/catalog"),

  projectSkillApps: (projectId: string) =>
    get<{ apps: SkillAppBinding[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/skill-apps`,
    ),

  bindProjectSkillApp: (
    projectId: string,
    body: { skill_id: string; slot?: string; enabled?: boolean },
  ) => put<{ ok: boolean }>(`/api/projects/${encodeURIComponent(projectId)}/skill-apps`, body),

  unbindProjectSkillApp: (projectId: string, skillId: string) =>
    del<{ ok: boolean }>(
      `/api/projects/${encodeURIComponent(projectId)}/skill-apps/${encodeURIComponent(skillId)}`,
    ),

  skillAppState: (projectId: string, skillId: string) =>
    get<{ state: unknown; brief: VisualBrief | null }>(
      `/api/projects/${encodeURIComponent(projectId)}/skill-apps/${encodeURIComponent(skillId)}/state`,
    ),

  putSkillAppState: (
    projectId: string,
    skillId: string,
    body: { state?: unknown; brief?: unknown },
  ) =>
    put<{ ok: boolean }>(
      `/api/projects/${encodeURIComponent(projectId)}/skill-apps/${encodeURIComponent(skillId)}/state`,
      body,
    ),

  submitSkillAppBrief: (
    projectId: string,
    body: { skill_id: string; brief: unknown; present_id?: string },
  ) =>
    post<{ ok: boolean; resumed?: boolean }>(
      `/api/projects/${encodeURIComponent(projectId)}/skill-apps/brief`,
      body,
    ),

  pendingSkillApps: (sessionId?: string) =>
    get<{ pending: PendingSkillAppPresent[] }>(
      sessionId
        ? `/api/skill-apps/pending?session_id=${encodeURIComponent(sessionId)}`
        : "/api/skill-apps/pending",
    ),

  skillAppAssetUrl: (skillId: string, path = "index.html") =>
    `/api/skill-apps/${encodeURIComponent(skillId)}/asset?path=${encodeURIComponent(path)}`,
};
