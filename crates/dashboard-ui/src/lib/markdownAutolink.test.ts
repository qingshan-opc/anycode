import { describe, expect, it } from "vitest";
import { boundMarkdownAutolinks } from "./markdownAutolink";

describe("boundMarkdownAutolinks", () => {
  it("keeps only the domain clickable when CJK punctuation follows www.", () => {
    const input =
      '百度已打开（www.baidu.com，标题"百度一下，你就知道"）。页面加载成功。';
    const out = boundMarkdownAutolinks(input);
    expect(out).toContain("[www.baidu.com](https://www.baidu.com)");
    expect(out).toContain('，标题"百度一下，你就知道"）。页面加载成功。');
    expect(out).not.toMatch(/\]\([^)]*标题/);
  });

  it("bounds https URLs before CJK commas", () => {
    const input = "已导航到 https://www.baidu.com，页面加载成功";
    const out = boundMarkdownAutolinks(input);
    expect(out).toBe(
      "已导航到 [www.baidu.com](https://www.baidu.com)，页面加载成功",
    );
  });

  it("does not rewrite URLs inside code fences", () => {
    const input = "```\nwww.baidu.com，标题\n```";
    expect(boundMarkdownAutolinks(input)).toBe(input);
  });

  it("leaves explicit markdown links alone", () => {
    const input = "[Baidu](https://www.baidu.com)";
    expect(boundMarkdownAutolinks(input)).toBe(input);
  });
});
