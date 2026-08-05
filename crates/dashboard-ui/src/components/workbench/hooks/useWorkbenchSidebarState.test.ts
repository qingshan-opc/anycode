import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  resetWorkbenchSidebarStateCache,
  workbenchSidebarStore,
} from "./useWorkbenchSidebarState";

const STORAGE_KEY = "anycode-workbench-sidebar";

function stubLocalStorage() {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => {
      store.set(k, v);
    },
    removeItem: (k: string) => {
      store.delete(k);
    },
  });
  return store;
}

describe("workbenchSidebarStore", () => {
  let backing: Map<string, string>;

  beforeEach(() => {
    backing = stubLocalStorage();
    resetWorkbenchSidebarStateCache();
  });

  it("hydrates from persisted state (legacy values still valid)", () => {
    backing.set(
      STORAGE_KEY,
      JSON.stringify({ expanded: true, activeTab: "terminal", panelWidth: 300 }),
    );
    const s = workbenchSidebarStore.getState();
    expect(s.expanded).toBe(true);
    expect(s.activeTab).toBe("terminal");
    expect(s.panelWidth).toBe(300);
  });

  it("drops unknown persisted tabs to the default", () => {
    backing.set(STORAGE_KEY, JSON.stringify({ expanded: true, activeTab: "gone", panelWidth: 300 }));
    expect(workbenchSidebarStore.getState().activeTab).toBe("files");
  });

  it("selectTab toggles collapse when re-selecting the active tab", () => {
    workbenchSidebarStore.openTab("browser");
    expect(workbenchSidebarStore.getState().expanded).toBe(true);
    workbenchSidebarStore.selectTab("browser");
    expect(workbenchSidebarStore.getState().expanded).toBe(false);
    workbenchSidebarStore.selectTab("browser");
    expect(workbenchSidebarStore.getState().expanded).toBe(true);
    workbenchSidebarStore.selectTab("artifacts");
    expect(workbenchSidebarStore.getState().activeTab).toBe("artifacts");
    expect(workbenchSidebarStore.getState().expanded).toBe(true);
  });

  it("clamps panel width and persists", () => {
    workbenchSidebarStore.setPanelWidth(100);
    expect(workbenchSidebarStore.getState().panelWidth).toBe(280);
    workbenchSidebarStore.setPanelWidth(9999);
    expect(workbenchSidebarStore.getState().panelWidth).toBe(720);
    const persisted = JSON.parse(backing.get(STORAGE_KEY)!) as { panelWidth: number };
    expect(persisted.panelWidth).toBe(720);
  });

  it("focus is transient (not persisted) and consumed once per tab", () => {
    workbenchSidebarStore.openTab("artifacts", { focus: "artifact-1" });
    expect(workbenchSidebarStore.consumeFocus("artifacts")).toBe("artifact-1");
    expect(workbenchSidebarStore.consumeFocus("artifacts")).toBeUndefined();
    const persisted = backing.get(STORAGE_KEY)!;
    expect(persisted).not.toContain("artifact-1");
  });

  it("consumeFocus ignores focus targeted at another tab", () => {
    workbenchSidebarStore.openTab("artifacts", { focus: "a1" });
    expect(workbenchSidebarStore.consumeFocus("browser")).toBeUndefined();
    // Still pending for the right tab.
    expect(workbenchSidebarStore.consumeFocus("artifacts")).toBe("a1");
  });

  it("markSeen records a per-tab watermark", () => {
    workbenchSidebarStore.markSeen("artifacts");
    const seen = workbenchSidebarStore.getState().lastSeen.artifacts;
    expect(typeof seen).toBe("string");
    expect(Number.isNaN(Date.parse(seen!))).toBe(false);
  });

  it("reset clears persistence and returns to defaults", () => {
    workbenchSidebarStore.openTab("browser");
    resetWorkbenchSidebarStateCache();
    expect(backing.has(STORAGE_KEY)).toBe(false);
    const s = workbenchSidebarStore.getState();
    expect(s.expanded).toBe(false);
    expect(s.activeTab).toBe("files");
  });
});
