export type SkillAppSlot = "dock" | "conversation" | "project";

export interface SkillSurface {
  id: string;
  title: string;
  entry: string;
  slots: SkillAppSlot[];
  default_slot: SkillAppSlot;
  lifecycle?: string;
  icon?: string | null;
  permissions?: {
    host_api?: string[];
    network?: boolean;
  };
}

export interface SkillAppBinding {
  skill_id: string;
  slot: SkillAppSlot | string;
  enabled: boolean;
  title?: string | null;
  icon?: string | null;
  has_ui: boolean;
  surface?: SkillSurface | null;
}

export interface SkillAppCatalogItem {
  skill_id: string;
  title: string;
  icon?: string | null;
  default_slot?: string;
  slots?: string[];
}

export interface PendingSkillAppPresent {
  present_id: string;
  session_id: string;
  user_turn_id?: number;
  skill_id: string;
  slot: string;
  wait_brief: boolean;
  push?: unknown;
  created_at: string;
  status: string;
}

export interface VisualBrief {
  skill_id: string;
  schema?: string;
  family?: string | null;
  templates?: string[];
  tokens?: Record<string, unknown>;
  density?: string | null;
  notes?: string | null;
  extra?: unknown;
}
