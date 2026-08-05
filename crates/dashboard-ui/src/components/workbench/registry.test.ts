import { describe, expect, it } from "vitest";
import { en } from "@/i18n/en";
import { zh } from "@/i18n/zh";
import {
  DEFAULT_WORKBENCH_TAB,
  isWorkbenchTab,
  WORKBENCH_PANELS,
  WORKBENCH_PANELS_ORDERED,
  workbenchPanelById,
} from "./registry";

function hasKey(tree: Record<string, unknown>, path: string): boolean {
  let cur: unknown = tree;
  for (const part of path.split(".")) {
    if (cur && typeof cur === "object" && part in (cur as object)) {
      cur = (cur as Record<string, unknown>)[part];
    } else {
      return false;
    }
  }
  return typeof cur === "string";
}

describe("workbench panel registry", () => {
  it("has unique ids and a valid default tab", () => {
    const ids = WORKBENCH_PANELS.map((p) => p.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain(DEFAULT_WORKBENCH_TAB);
  });

  it("covers the legacy tab set (localStorage back-compat)", () => {
    for (const legacy of ["files", "browser", "terminal", "artifacts", "plan"]) {
      expect(isWorkbenchTab(legacy)).toBe(true);
    }
    expect(isWorkbenchTab("nope")).toBe(false);
    expect(isWorkbenchTab(undefined)).toBe(false);
  });

  it("orders panels by ascending order field", () => {
    const orders = WORKBENCH_PANELS_ORDERED.map((p) => p.order);
    expect([...orders].sort((a, b) => a - b)).toEqual(orders);
  });

  it("every titleKey exists in en and zh messages", () => {
    for (const panel of WORKBENCH_PANELS) {
      expect(hasKey(en as unknown as Record<string, unknown>, panel.titleKey)).toBe(true);
      expect(hasKey(zh as unknown as Record<string, unknown>, panel.titleKey)).toBe(true);
    }
  });

  it("workbenchPanelById resolves every registered panel", () => {
    for (const panel of WORKBENCH_PANELS) {
      expect(workbenchPanelById(panel.id).id).toBe(panel.id);
    }
    expect(() => workbenchPanelById("nope" as never)).toThrow();
  });
});
