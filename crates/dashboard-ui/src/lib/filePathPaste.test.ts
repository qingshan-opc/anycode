import { describe, expect, it } from "vitest";
import { pastedFilePaths } from "./filePathPaste";

describe("pastedFilePaths", () => {
  it("detects a single absolute path", () => {
    expect(pastedFilePaths("/Users/x/notes/report.md")).toEqual([
      "/Users/x/notes/report.md",
    ]);
  });

  it("detects tilde and multi-line path lists", () => {
    expect(pastedFilePaths("~/a.md\n/Users/x/b.md\n")).toEqual([
      "~/a.md",
      "/Users/x/b.md",
    ]);
  });

  it("detects windows drive paths", () => {
    expect(pastedFilePaths("C:\\Users\\x\\a.md")).toEqual(["C:\\Users\\x\\a.md"]);
  });

  it("keeps paths containing spaces", () => {
    expect(pastedFilePaths("/Users/x/My Documents/a.md")).toEqual([
      "/Users/x/My Documents/a.md",
    ]);
  });

  it("returns null for prose and mixed content", () => {
    expect(pastedFilePaths("hello world")).toBeNull();
    expect(pastedFilePaths("看一下 /Users/x/a.md 这个文件")).toBeNull();
    expect(pastedFilePaths("/Users/x/a.md\n这是一句话")).toBeNull();
    expect(pastedFilePaths("https://example.com/a")).toBeNull();
    expect(pastedFilePaths("")).toBeNull();
    expect(pastedFilePaths("/")).toBeNull();
  });

  it("returns null for relative paths", () => {
    expect(pastedFilePaths("src/main.ts")).toBeNull();
    expect(pastedFilePaths("./a.md")).toBeNull();
  });
});
