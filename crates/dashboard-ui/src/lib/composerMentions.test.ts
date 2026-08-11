import { describe, expect, it } from "vitest";
import { extractMentionedSkillIds } from "./composerMentions";

describe("extractMentionedSkillIds", () => {
  const known = ["anycode-ppt", "anycode-xlsx", "mindmap"];

  it("extracts mentioned known skills in order", () => {
    expect(
      extractMentionedSkillIds("用 @anycode-ppt 做封面，再用 @mindmap 汇总", known),
    ).toEqual(["anycode-ppt", "mindmap"]);
  });

  it("dedupes repeated mentions", () => {
    expect(extractMentionedSkillIds("@anycode-ppt 然后 @anycode-ppt", known)).toEqual([
      "anycode-ppt",
    ]);
  });

  it("ignores unknown mentions and emails", () => {
    expect(extractMentionedSkillIds("联系 a@b.com 或 @unknown", known)).toEqual([]);
  });

  it("returns empty for no mention or no known ids", () => {
    expect(extractMentionedSkillIds("没有提及", known)).toEqual([]);
    expect(extractMentionedSkillIds("@anycode-ppt", [])).toEqual([]);
  });
});
