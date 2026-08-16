import { describe, expect, it } from "vitest";
import {
  buildSkillAppContinuationPrompt,
  originalPromptFromPush,
} from "./skillAppPrompt";

describe("skillAppPrompt", () => {
  it("reads user_prompt from auto-present push", () => {
    expect(originalPromptFromPush({ user_prompt: " 帮我做个ppt " })).toBe("帮我做个ppt");
    expect(originalPromptFromPush({})).toBeUndefined();
  });

  it("keeps the original task and forbids echoing the brief", () => {
    const text = buildSkillAppContinuationPrompt({
      skillId: "anycode-ppt",
      originalPrompt: "帮我做个ai投标的ppt",
      brief: { family: "fde-editorial" },
    });
    expect(text).toContain("帮我做个ai投标的ppt");
    expect(text).toContain("[Host VisualBrief for `anycode-ppt`");
    expect(text).toContain("fde-editorial");
    expect(text).not.toContain("VisualBrief locked");
    expect(text).toContain("Do not echo this block");
    expect(text).toContain("Infer outline");
    expect(text).not.toContain("copy templates, rewrite");
  });

  it("uses video instructions for anycode-video briefs", () => {
    const text = buildSkillAppContinuationPrompt({
      skillId: "anycode-video",
      originalPrompt: "帮我做个抖音短视频",
      brief: {
        family: "frame-glitch-title",
        templates: ["frame-glitch-title", "frame-logo-outro"],
        extra: { aspect: "9:16", duration_sec: 4 },
      },
    });
    expect(text).toContain("帮我做个抖音短视频");
    expect(text).toContain("[Host VisualBrief for `anycode-video`");
    expect(text).toContain("html-video template");
    expect(text).toContain("brief.templates[]");
    expect(text).toContain("run video/");
    expect(text).toContain("Do not call GenerateVideo");
    expect(text).not.toContain("Infer outline");
    expect(text).toContain("frame-glitch-title");
  });
});
