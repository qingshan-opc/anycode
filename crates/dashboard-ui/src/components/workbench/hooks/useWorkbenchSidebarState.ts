import { useCallback, useSyncExternalStore } from "react";
import type { WorkbenchTab } from "@/api/types/workbench";
import { DEFAULT_WORKBENCH_TAB, isWorkbenchTab } from "../registry";

const STORAGE_KEY = "anycode-workbench-sidebar";

export type WorkbenchFocus = { tab: WorkbenchTab; payload: unknown };

export type WorkbenchSidebarState = {
  expanded: boolean;
  activeTab: WorkbenchTab;
  panelWidth: number;
  /** Transient focus request for a panel (not persisted); consumed once. */
  focus: WorkbenchFocus | null;
  /** Per-tab "seen" watermark (ISO timestamp) for badge unread semantics. */
  lastSeen: Partial<Record<WorkbenchTab, string>>;
};

const DEFAULT: WorkbenchSidebarState = {
  expanded: false,
  activeTab: DEFAULT_WORKBENCH_TAB,
  panelWidth: 420,
  focus: null,
  lastSeen: {},
};

const PANEL_WIDTH_MIN = 280;
const PANEL_WIDTH_MAX = 720;

function clampPanelWidth(w: unknown): number {
  const n = typeof w === "number" && Number.isFinite(w) ? w : DEFAULT.panelWidth;
  return Math.min(PANEL_WIDTH_MAX, Math.max(PANEL_WIDTH_MIN, n));
}

function sanitize(raw: Partial<WorkbenchSidebarState>): WorkbenchSidebarState {
  return {
    expanded: raw.expanded ?? DEFAULT.expanded,
    activeTab: isWorkbenchTab(raw.activeTab) ? raw.activeTab : DEFAULT.activeTab,
    panelWidth: clampPanelWidth(raw.panelWidth),
    focus: null,
    lastSeen:
      raw.lastSeen && typeof raw.lastSeen === "object"
        ? Object.fromEntries(
            Object.entries(raw.lastSeen).filter(
              ([k, v]) => isWorkbenchTab(k) && typeof v === "string",
            ),
          )
        : {},
  };
}

function persistedFields(s: WorkbenchSidebarState) {
  const { focus: _focus, ...rest } = s;
  return rest;
}

/* ------------------------------------------------------------------ */
/* Module-level singleton store. Every useWorkbenchSidebarState()       */
/* instance subscribes to this one store, so all consumers stay in sync */
/* (previously each hook had its own useState + localStorage write).    */
/* ------------------------------------------------------------------ */

type Listener = () => void;

function hasLocalStorage(): boolean {
  try {
    return typeof localStorage !== "undefined";
  } catch {
    return false;
  }
}

function readPersisted(): WorkbenchSidebarState {
  if (!hasLocalStorage()) return DEFAULT;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT;
    return sanitize(JSON.parse(raw) as Partial<WorkbenchSidebarState>);
  } catch {
    return DEFAULT;
  }
}

const listeners = new Set<Listener>();
let state: WorkbenchSidebarState = DEFAULT;
let initialized = false;

function ensureInit(): void {
  if (initialized) return;
  initialized = true;
  state = readPersisted();
  if (typeof window !== "undefined") {
    window.addEventListener("storage", (e) => {
      if (e.key !== STORAGE_KEY || e.newValue == null) return;
      try {
        // Merge external (other tab) writes; keep transient focus local.
        const external = sanitize(JSON.parse(e.newValue) as Partial<WorkbenchSidebarState>);
        setState({ ...external, focus: state.focus }, /* persist */ false);
      } catch {
        /* ignore malformed external writes */
      }
    });
  }
}

function setState(next: WorkbenchSidebarState, persist = true): void {
  state = next;
  if (persist && hasLocalStorage()) {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(persistedFields(next)));
    } catch {
      /* storage full / unavailable — keep in-memory state */
    }
  }
  for (const l of listeners) l();
}

function update(patch: Partial<WorkbenchSidebarState>): void {
  setState({ ...state, ...patch });
}

/**
 * Plain store API — usable without React (and directly testable). The hook
 * below is a thin useSyncExternalStore wrapper over this object.
 */
export const workbenchSidebarStore = {
  getState(): WorkbenchSidebarState {
    ensureInit();
    return state;
  },
  selectTab(tab: WorkbenchTab): void {
    ensureInit();
    if (state.expanded && state.activeTab === tab) {
      update({ expanded: false });
    } else {
      update({ expanded: true, activeTab: tab });
    }
  },
  setExpanded(expanded: boolean): void {
    ensureInit();
    update({ expanded });
  },
  setPanelWidth(panelWidth: number): void {
    ensureInit();
    update({ panelWidth: clampPanelWidth(panelWidth) });
  },
  openTab(tab: WorkbenchTab, opts?: { focus?: unknown }): void {
    ensureInit();
    update({
      expanded: true,
      activeTab: tab,
      focus: opts && "focus" in opts ? { tab, payload: opts.focus } : state.focus,
    });
  },
  /** Consume (read + clear) a pending focus request for `tab`. */
  consumeFocus(tab: WorkbenchTab): unknown {
    ensureInit();
    if (!state.focus || state.focus.tab !== tab) return undefined;
    const payload = state.focus.payload;
    update({ focus: null });
    return payload;
  },
  /** Mark a tab as seen up to now (badge unread semantics). */
  markSeen(tab: WorkbenchTab): void {
    ensureInit();
    update({ lastSeen: { ...state.lastSeen, [tab]: new Date().toISOString() } });
  },
};

function subscribe(listener: Listener): () => void {
  ensureInit();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useWorkbenchSidebarState() {
  const snapshot = useSyncExternalStore(subscribe, workbenchSidebarStore.getState, () => DEFAULT);

  const selectTab = useCallback((tab: WorkbenchTab) => workbenchSidebarStore.selectTab(tab), []);
  const setExpanded = useCallback(
    (expanded: boolean) => workbenchSidebarStore.setExpanded(expanded),
    [],
  );
  const setPanelWidth = useCallback(
    (panelWidth: number) => workbenchSidebarStore.setPanelWidth(panelWidth),
    [],
  );
  const openTab = useCallback(
    (tab: WorkbenchTab, opts?: { focus?: unknown }) => workbenchSidebarStore.openTab(tab, opts),
    [],
  );
  const consumeFocus = useCallback(
    (tab: WorkbenchTab) => workbenchSidebarStore.consumeFocus(tab),
    [],
  );
  const markSeen = useCallback((tab: WorkbenchTab) => workbenchSidebarStore.markSeen(tab), []);

  return {
    ...snapshot,
    selectTab,
    setExpanded,
    setPanelWidth,
    openTab,
    consumeFocus,
    markSeen,
  };
}

export function resetWorkbenchSidebarStateCache(): void {
  if (hasLocalStorage()) {
    try {
      localStorage.removeItem(STORAGE_KEY);
    } catch {
      /* ignore */
    }
  }
  state = DEFAULT;
  initialized = false;
  for (const l of listeners) l();
}
