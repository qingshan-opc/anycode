export type FsEntryKind = "file" | "dir" | "symlink";

export interface FsEntry {
  name: string;
  path: string;
  kind: FsEntryKind;
  size?: number;
}

export interface FsReadResult {
  path: string;
  content: string;
  truncated: boolean;
  size: number;
  mime_hint: string;
}

export interface BrowserState {
  url: string;
  title: string;
  lock?: "idle" | "agent" | "user";
}

export interface BrowserScreenshot {
  image_base64: string;
  viewport: { width: number; height: number };
}

export interface BrowserSessionInfo {
  session_id: string;
  project_id: string;
  conversation_id?: string | null;
}

export interface BrowserHitTestResult {
  x: number;
  y: number;
  tag: string;
  id?: string | null;
  classes: string[];
  text?: string | null;
  css_selector: string;
  xpath?: string | null;
}

export interface BrowserDesignInspectState {
  enabled: boolean;
  hover?: BrowserHitTestResult | null;
  pick?: BrowserHitTestResult | null;
}

export interface ScreencastMetadata {
  offset_top: number;
  page_scale_factor: number;
  device_width: number;
  device_height: number;
  scroll_offset_x: number;
  scroll_offset_y: number;
  timestamp?: number | null;
}

export interface TerminalSessionInfo {
  session_id: string;
  project_id: string;
  conversation_id: string;
}

// The tab union is derived from the panel registry (single source of truth).
// Type-only re-export — no runtime dependency on the registry module.
export type { WorkbenchTab } from "@/components/workbench/registry";

export interface GitStatusSummary {
  is_repo: boolean;
  branch: string | null;
  insertions: number;
  deletions: number;
  changed_files: number;
  ahead: number;
  behind: number;
  has_upstream: boolean;
  has_changes: boolean;
}

export type GitChangeKind =
  | "modified"
  | "added"
  | "deleted"
  | "renamed"
  | "untracked"
  | "type_changed";

export interface GitFileChange {
  path: string;
  old_path: string;
  kind: GitChangeKind;
  staged: boolean;
  status: string;
  insertions: number;
  deletions: number;
}

export interface GitFileDiff {
  path: string;
  kind: GitChangeKind;
  diff: string;
  insertions: number;
  deletions: number;
}

export interface GitBranchInfo {
  name: string;
  current: boolean;
  remote: boolean;
}

export interface GitLogEntry {
  hash: string;
  short_hash: string;
  author: string;
  date: string;
  subject: string;
}

export type PlanStatus =
  | "pending"
  | "in_progress"
  | "completed"
  | "blocked"
  | "failed"
  | "cancelled";

export type PlanNodeKind = "phase" | "task" | "verify" | "checkpoint";

export interface PlanNode {
  id: string;
  title: string;
  status: PlanStatus;
  children?: PlanNode[];
  detail?: string | null;
  kind?: PlanNodeKind | null;
}

export interface PlanTree {
  /** Guidance prose at the top of the Markdown plan document. */
  prose?: string;
  roots: PlanNode[];
}

export interface SessionPlanTreeResponse {
  tree: PlanTree;
  updated_at: string | null;
}
