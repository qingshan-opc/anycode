import type {
  BrowserDesignInspectState,
  BrowserHitTestResult,
  BrowserScreenshot,
  BrowserSessionInfo,
  BrowserState,
  FsEntry,
  FsReadResult,
  GitBranchInfo,
  GitChangeKind,
  GitFileChange,
  GitFileDiff,
  GitLogEntry,
  GitStatusSummary,
  TerminalSessionInfo,
} from "../types/workbench";
import { del, get, post, apiWebSocketUrl } from "../http";

export const workbenchClient = {
  listProjectFs: (projectId: string, path = "") =>
    get<{ entries: FsEntry[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/fs/list?path=${encodeURIComponent(path)}`,
    ),

  readProjectFs: (projectId: string, path: string, maxBytes = 512 * 1024) =>
    get<{ file: FsReadResult }>(
      `/api/projects/${encodeURIComponent(projectId)}/fs/read?path=${encodeURIComponent(path)}&max_bytes=${maxBytes}`,
    ),

  createBrowserSession: (
    projectId: string,
    conversationId?: string | null,
    viewport?: { width: number; height: number; device_scale_factor?: number } | null,
  ) =>
    post<{ session: BrowserSessionInfo }>("/api/workbench/browser/sessions", {
      project_id: projectId,
      conversation_id: conversationId ?? undefined,
      viewport: viewport ?? undefined,
    }),

  setBrowserViewport: (
    sessionId: string,
    width: number,
    height: number,
    deviceScaleFactor?: number,
  ) =>
    post<{ viewport: { width: number; height: number } }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/viewport`,
      {
        width,
        height,
        device_scale_factor: deviceScaleFactor,
      },
    ),

  navigateBrowser: (sessionId: string, url: string) =>
    post<{ state: BrowserState }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/navigate`,
      { url },
    ),

  browserState: (sessionId: string) =>
    get<{ state: BrowserState }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/state`,
    ),

  browserScreenshot: (sessionId: string) =>
    get<{ screenshot: BrowserScreenshot }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/screenshot`,
    ),

  browserHitTest: (sessionId: string, x: number, y: number) =>
    post<{ hit: BrowserHitTestResult | null }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/hit-test`,
      { x, y },
    ),

  browserDesignMode: (sessionId: string, enabled: boolean) =>
    post<{ ok: boolean; enabled: boolean }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/design-mode`,
      { enabled },
    ),

  browserDesignInspect: (sessionId: string) =>
    get<{ inspect: BrowserDesignInspectState }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/design-inspect`,
    ),

  deleteBrowserSession: (sessionId: string) =>
    del<{ ok: boolean }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}`,
    ),

  browserLock: (sessionId: string, lock: "user" | "agent" | "idle") =>
    post<{ lock: string }>(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/lock`,
      { lock },
    ),

  browserStreamUrl: (sessionId: string) =>
    apiWebSocketUrl(
      `/api/workbench/browser/sessions/${encodeURIComponent(sessionId)}/stream`,
    ),

  browserStatus: () =>
    get<{
      enabled: boolean;
      ready: boolean;
      bundled: boolean;
      chromium_ready: boolean;
      doctor_message?: string;
    }>("/api/workbench/browser/status"),

  createTerminalSession: (projectId: string, conversationId: string) =>
    post<{ session: TerminalSessionInfo }>("/api/workbench/terminal/sessions", {
      project_id: projectId,
      conversation_id: conversationId,
    }),

  listTerminalSessions: (projectId: string, conversationId: string) =>
    get<{ sessions: TerminalSessionInfo[] }>(
      `/api/workbench/terminal/sessions?project_id=${encodeURIComponent(projectId)}&conversation_id=${encodeURIComponent(conversationId)}`,
    ),

  deleteTerminalSession: (sessionId: string) =>
    del<{ ok: boolean }>(
      `/api/workbench/terminal/sessions/${encodeURIComponent(sessionId)}`,
    ),

  terminalSessionWsUrl: (sessionId: string) =>
    apiWebSocketUrl(
      `/api/workbench/terminal/sessions/${encodeURIComponent(sessionId)}/ws`,
    ),

  projectGitStatus: (projectId: string) =>
    get<{ git: GitStatusSummary }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/status`,
    ),

  projectGitChanges: (projectId: string) =>
    get<{ changes: GitFileChange[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/changes`,
    ),

  projectGitFileDiff: (projectId: string, path: string, kind: GitChangeKind) =>
    get<{ diff: GitFileDiff }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/diff?path=${encodeURIComponent(path)}&kind=${encodeURIComponent(kind)}`,
    ),

  projectGitCommit: (projectId: string, body?: { message?: string }) =>
    post<{ ok: boolean }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/commit`,
      body ?? {},
    ),

  projectGitPush: (projectId: string) =>
    post<{ ok: boolean; detail?: string }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/push`,
      {},
    ),

  projectGitBranches: (projectId: string) =>
    get<{ branches: GitBranchInfo[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/branches`,
    ),

  projectGitCheckout: (projectId: string, branch: string, force = false) =>
    post<{ ok: boolean } | { error: "dirty_tree"; files: string[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/checkout`,
      { branch, force },
      { acceptStatuses: [409] },
    ),

  projectGitCreateBranch: (projectId: string, name: string, checkout = true) =>
    post<{ ok: boolean }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/branch`,
      { name, checkout },
    ),

  projectGitLog: (projectId: string, limit = 30, offset = 0) =>
    get<{ commits: GitLogEntry[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/log?limit=${limit}&offset=${offset}`,
    ),

  projectGitCommitDiff: (projectId: string, hash: string) =>
    get<{ diff: GitFileDiff }>(
      `/api/projects/${encodeURIComponent(projectId)}/git/commit-diff?hash=${encodeURIComponent(hash)}`,
    ),
};
