import { lazy, type ComponentType, type LazyExoticComponent } from "react";

/**
 * Standard props every workbench panel receives. Panels ignore what they do
 * not need; registry wrappers adapt these to each panel's own prop shape.
 */
export type PanelProps = {
  projectId: string | null;
  sessionId: string;
  /** Whether the dock is currently expanded (panel visible). */
  active: boolean;
  /** Whether the session is currently running. */
  isRunning: boolean;
  /** Collapse the workbench dock (e.g. after starting a plan build). */
  collapse: () => void;
};

export type WorkbenchPanelDef = {
  id: string;
  icon: string;
  titleKey: string;
  order: number;
  /** Panel requires the session to have a project root. */
  needsProject?: boolean;
  component: LazyExoticComponent<ComponentType<PanelProps>>;
};

/**
 * The Browser panel is preloaded at registry import time: auto-open depends on
 * the panel mounting fast enough to create the CEF host surface before Agent
 * Browser* tools attach.
 */
const browserPanelLoader = () => import("./panels/BrowserPanel");
if (typeof window !== "undefined") {
  void browserPanelLoader();
}

const FilesPanelLazy = lazy(() =>
  import("./panels/FilesPanel").then((m) => ({
    default: (p: PanelProps) => <m.FilesPanel projectId={p.projectId!} />,
  })),
);

const BrowserPanelLazy = lazy(() =>
  browserPanelLoader().then((m) => ({
    default: (p: PanelProps) => (
      <m.BrowserPanel
        projectId={p.projectId!}
        conversationSessionId={p.sessionId}
        active={p.active}
      />
    ),
  })),
);

const TerminalPanelLazy = lazy(() =>
  import("./panels/TerminalPanel").then((m) => ({
    default: (p: PanelProps) => (
      <m.TerminalPanel
        projectId={p.projectId!}
        conversationSessionId={p.sessionId}
        active={p.active}
      />
    ),
  })),
);

const PlanTreePanelLazy = lazy(() =>
  import("./panels/PlanTreePanel").then((m) => ({
    default: (p: PanelProps) => (
      <m.PlanTreePanel
        sessionId={p.sessionId}
        isRunning={p.isRunning}
        onBuildStarted={p.collapse}
      />
    ),
  })),
);

const ArtifactsPanelLazy = lazy(() =>
  import("./panels/ArtifactsPanel").then((m) => ({
    default: (p: PanelProps) => (
      <m.ArtifactsPanel sessionId={p.sessionId} isRunning={p.isRunning} />
    ),
  })),
);

const SkillAppPanelLazy = lazy(() =>
  import("./panels/SkillAppPanel").then((m) => ({
    default: (p: PanelProps) => (
      <m.SkillAppPanel
        projectId={p.projectId}
        sessionId={p.sessionId}
        active={p.active}
      />
    ),
  })),
);

/**
 * Single source of truth for the workbench side dock. Adding a panel =
 * adding one entry here (plus its i18n titleKey); rail icons, header icons,
 * panel title, tab validation and the panel switch all derive from this list.
 */
export const WORKBENCH_PANELS = [
  {
    id: "files",
    icon: "folder",
    titleKey: "workbench.tabFiles",
    order: 10,
    needsProject: true,
    component: FilesPanelLazy,
  },
  {
    id: "browser",
    icon: "language",
    titleKey: "workbench.tabBrowser",
    order: 20,
    needsProject: true,
    component: BrowserPanelLazy,
  },
  {
    id: "terminal",
    icon: "terminal",
    titleKey: "workbench.tabTerminal",
    order: 30,
    needsProject: true,
    component: TerminalPanelLazy,
  },
  {
    id: "plan",
    icon: "account_tree",
    titleKey: "workbench.tabPlan",
    order: 40,
    component: PlanTreePanelLazy,
  },
  {
    id: "artifacts",
    icon: "inventory_2",
    titleKey: "workbench.tabArtifacts",
    order: 50,
    component: ArtifactsPanelLazy,
  },
  {
    id: "skillApp",
    icon: "dashboard_customize",
    titleKey: "workbench.tabSkillApp",
    order: 60,
    needsProject: true,
    component: SkillAppPanelLazy,
  },
] as const satisfies readonly WorkbenchPanelDef[];

export type WorkbenchTab = (typeof WORKBENCH_PANELS)[number]["id"];

export const DEFAULT_WORKBENCH_TAB: WorkbenchTab = "files";

export function workbenchPanelById(id: WorkbenchTab): WorkbenchPanelDef {
  const def = WORKBENCH_PANELS.find((p) => p.id === id);
  if (!def) throw new Error(`unknown workbench panel: ${id}`);
  return def;
}

export function isWorkbenchTab(value: unknown): value is WorkbenchTab {
  return WORKBENCH_PANELS.some((p) => p.id === value);
}

/** Panels sorted for display (rail / header icons). Keeps literal id types. */
export const WORKBENCH_PANELS_ORDERED = [...WORKBENCH_PANELS].sort(
  (a, b) => a.order - b.order,
);
