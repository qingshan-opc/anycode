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

  it("moveToConversationTab appends, dedupes, selects, and collapses the dock showing that tab", () => {
    workbenchSidebarStore.openTab("browser"); // dock expanded on browser
    workbenchSidebarStore.moveToConversationTab("browser");
    let s = workbenchSidebarStore.getState();
    expect(s.tabbedPanels).toEqual(["browser"]);
    expect(s.conversationTab).toBe("browser");
    expect(s.expanded).toBe(false); // no double-mount

    // Dock showing a different tab stays expanded.
    workbenchSidebarStore.openTab("files");
    workbenchSidebarStore.moveToConversationTab("terminal");
    s = workbenchSidebarStore.getState();
    expect(s.tabbedPanels).toEqual(["browser", "terminal"]);
    expect(s.conversationTab).toBe("terminal");
    expect(s.expanded).toBe(true);
    expect(s.activeTab).toBe("files");

    workbenchSidebarStore.moveToConversationTab("browser");
    expect(workbenchSidebarStore.getState().tabbedPanels).toEqual(["browser", "terminal"]);
  });

  it("openTab on a tabbed panel selects the conversation tab without touching the dock", () => {
    workbenchSidebarStore.moveToConversationTab("browser");
    workbenchSidebarStore.setExpanded(false);
    workbenchSidebarStore.openTab("browser");
    const s = workbenchSidebarStore.getState();
    expect(s.conversationTab).toBe("browser");
    expect(s.expanded).toBe(false);
    expect(s.activeTab).toBe("files");
  });

  it("openTab on a non-tabbed panel keeps legacy dock behavior", () => {
    workbenchSidebarStore.moveToConversationTab("browser");
    workbenchSidebarStore.openTab("files");
    const s = workbenchSidebarStore.getState();
    expect(s.expanded).toBe(true);
    expect(s.activeTab).toBe("files");
    expect(s.conversationTab).toBe("browser");
  });

  it("selectTab toggles a tabbed panel between tab and chat", () => {
    workbenchSidebarStore.moveToConversationTab("browser");
    workbenchSidebarStore.selectConversationTab("chat");
    workbenchSidebarStore.selectTab("browser");
    expect(workbenchSidebarStore.getState().conversationTab).toBe("browser");
    workbenchSidebarStore.selectTab("browser");
    expect(workbenchSidebarStore.getState().conversationTab).toBe("chat");
    expect(workbenchSidebarStore.getState().expanded).toBe(false);
  });

  it("moveToDock removes the panel, resets conversationTab, and reveals the dock", () => {
    workbenchSidebarStore.moveToConversationTab("browser");
    workbenchSidebarStore.moveToDock("browser");
    const s = workbenchSidebarStore.getState();
    expect(s.tabbedPanels).toEqual([]);
    expect(s.conversationTab).toBe("chat");
    expect(s.expanded).toBe(true);
    expect(s.activeTab).toBe("browser");
  });

  it("selectConversationTab rejects ids that are not tabbed", () => {
    workbenchSidebarStore.selectConversationTab("browser");
    expect(workbenchSidebarStore.getState().conversationTab).toBe("chat");
    workbenchSidebarStore.moveToConversationTab("browser");
    workbenchSidebarStore.selectConversationTab("chat");
    expect(workbenchSidebarStore.getState().conversationTab).toBe("chat");
  });

  it("hydrates tabbedPanels/conversationTab, drops dangling conversationTab", () => {
    backing.set(
      STORAGE_KEY,
      JSON.stringify({ tabbedPanels: ["browser", "bogus"], conversationTab: "browser" }),
    );
    let s = workbenchSidebarStore.getState();
    expect(s.tabbedPanels).toEqual(["browser"]);
    expect(s.conversationTab).toBe("browser");

    resetWorkbenchSidebarStateCache();
    backing.set(STORAGE_KEY, JSON.stringify({ tabbedPanels: [], conversationTab: "browser" }));
    s = workbenchSidebarStore.getState();
    expect(s.conversationTab).toBe("chat");
  });

  it("legacy persisted JSON without new fields gets defaults", () => {
    backing.set(STORAGE_KEY, JSON.stringify({ expanded: true, activeTab: "files" }));
    const s = workbenchSidebarStore.getState();
    expect(s.tabbedPanels).toEqual([]);
    expect(s.conversationTab).toBe("chat");
  });

  it("hydration collapses a dock expanded on a tabbed panel (no double-mount)", () => {
    // Poisoned/seeded state: tabbed + dock-expanded on the same tab would mount
    // BrowserPanel twice; the second instance's syncShow resurrects closed CEF
    // tabs via show_in_parent.
    backing.set(
      STORAGE_KEY,
      JSON.stringify({
        expanded: true,
        activeTab: "browser",
        tabbedPanels: ["browser"],
        conversationTab: "browser",
      }),
    );
    const s = workbenchSidebarStore.getState();
    expect(s.expanded).toBe(false);
    expect(s.activeTab).toBe("browser");
    expect(s.conversationTab).toBe("browser");
  });

  it("update guard: setting expanded on a tabbed activeTab is collapsed centrally", () => {
    workbenchSidebarStore.moveToConversationTab("browser");
    // Force the illegal combination through a raw patch path (openTab on the
    // tabbed panel must not re-expand the dock either).
    workbenchSidebarStore.openTab("browser");
    expect(workbenchSidebarStore.getState().expanded).toBe(false);
  });

  it("persists tabbedPanels and conversationTab", () => {
    workbenchSidebarStore.moveToConversationTab("browser");
    const persisted = JSON.parse(backing.get(STORAGE_KEY)!) as {
      tabbedPanels: string[];
      conversationTab: string;
    };
    expect(persisted.tabbedPanels).toEqual(["browser"]);
    expect(persisted.conversationTab).toBe("browser");
  });

  it("collapseTab collapses the dock when the tab is dock-expanded", () => {
    workbenchSidebarStore.openTab("browser");
    workbenchSidebarStore.collapseTab("browser");
    const s = workbenchSidebarStore.getState();
    expect(s.expanded).toBe(false);
    expect(s.activeTab).toBe("browser"); // dock remembers the tab for reopen
  });

  it("collapseTab falls back to chat when the tab lives as a conversation tab", () => {
    workbenchSidebarStore.moveToConversationTab("browser");
    workbenchSidebarStore.collapseTab("browser");
    const s = workbenchSidebarStore.getState();
    expect(s.conversationTab).toBe("chat");
    expect(s.tabbedPanels).toEqual(["browser"]); // tab entry stays for reopen
    expect(s.expanded).toBe(false);
  });

  it("closeConversationTab removes the tab and returns to chat without expanding dock", () => {
    workbenchSidebarStore.moveToConversationTab("skillApp");
    workbenchSidebarStore.closeConversationTab("skillApp");
    const s = workbenchSidebarStore.getState();
    expect(s.conversationTab).toBe("chat");
    expect(s.tabbedPanels).toEqual([]);
    expect(s.expanded).toBe(false);
  });

  it("collapseTab is a no-op for a non-visible tab", () => {
    workbenchSidebarStore.openTab("files");
    workbenchSidebarStore.collapseTab("browser");
    let s = workbenchSidebarStore.getState();
    expect(s.expanded).toBe(true);
    expect(s.activeTab).toBe("files");

    workbenchSidebarStore.moveToConversationTab("browser");
    workbenchSidebarStore.selectConversationTab("chat");
    workbenchSidebarStore.openTab("files");
    workbenchSidebarStore.collapseTab("browser");
    s = workbenchSidebarStore.getState();
    expect(s.conversationTab).toBe("chat");
    expect(s.expanded).toBe(true);
    expect(s.activeTab).toBe("files");
  });
});
