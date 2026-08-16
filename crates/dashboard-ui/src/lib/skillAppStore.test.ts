import { describe, expect, it, beforeEach } from "vitest";
import { skillAppStore } from "./skillAppStore";

describe("skillAppStore", () => {
  beforeEach(() => {
    skillAppStore.dismiss();
  });

  it("setFocus stores the active Skill App", () => {
    skillAppStore.setFocus({
      skillId: "anycode-ppt",
      presentId: "sa_1",
      waitBrief: true,
    });
    expect(skillAppStore.getFocus()?.skillId).toBe("anycode-ppt");
    expect(skillAppStore.getFocus()?.waitBrief).toBe(true);
  });

  it("dismiss clears focus so the host can unmount the iframe", () => {
    skillAppStore.setFocus({ skillId: "anycode-ppt", presentId: "sa_1" });
    skillAppStore.dismiss();
    expect(skillAppStore.getFocus()).toBeNull();
  });
});
