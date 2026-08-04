import { describe, expect, it } from "vitest";
import {
  isExternalHref,
  isLocalPathHref,
  resolveMarkdownLocalPath,
} from "./markdownLinkPath";

describe("markdownLinkPath", () => {
  it("classifies external vs local hrefs", () => {
    expect(isExternalHref("https://example.com")).toBe(true);
    expect(isExternalHref("mailto:a@b.c")).toBe(true);
    expect(isLocalPathHref("https://example.com")).toBe(false);
    expect(isLocalPathHref("crates/agent/src/lib.rs")).toBe(true);
    expect(isLocalPathHref("./MEMORY.md")).toBe(true);
    expect(isLocalPathHref("../docs/a.md")).toBe(true);
    expect(isLocalPathHref("/tmp/x.md")).toBe(true);
    expect(isLocalPathHref("#section")).toBe(false);
  });

  it("resolves relative paths against project root", () => {
    expect(resolveMarkdownLocalPath("docs/a.md", "/proj")).toBe("/proj/docs/a.md");
    expect(resolveMarkdownLocalPath("./MEMORY.md", "/proj")).toBe("/proj/MEMORY.md");
    expect(resolveMarkdownLocalPath("/abs/x.md", "/proj")).toBe("/abs/x.md");
    expect(resolveMarkdownLocalPath("file:///Users/me/x.md", null)).toBe("/Users/me/x.md");
  });
});
