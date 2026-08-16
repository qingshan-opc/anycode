import { describe, expect, it } from "vitest";
import { apiUrl } from "@/api/http";
import { projectFsRawRelPath, projectFsRawUrl } from "./projectFsUrl";

describe("projectFsRawUrl", () => {
  it("uses path-style URLs so HTML relative links stay in the deck directory", () => {
    expect(projectFsRawUrl("proj", "slides/index.html")).toBe(
      apiUrl("/api/projects/proj/fs/raw/slides/index.html"),
    );
    expect(projectFsRawUrl("proj", "slides/01-cover.html")).toBe(
      apiUrl("/api/projects/proj/fs/raw/slides/01-cover.html"),
    );
  });

  it("strips project root from absolute paths", () => {
    expect(
      projectFsRawRelPath("/Users/me/ppt/slides/01.html", "/Users/me/ppt"),
    ).toBe("slides/01.html");
    expect(projectFsRawUrl("proj", "/Users/me/ppt/slides/01.html", "/Users/me/ppt")).toBe(
      apiUrl("/api/projects/proj/fs/raw/slides/01.html"),
    );
  });

  it("keeps query form when an absolute path cannot be made relative", () => {
    expect(projectFsRawRelPath("/tmp/outside.html", "/Users/me/ppt")).toBeNull();
    expect(projectFsRawUrl("proj", "/tmp/outside.html")).toContain("fs/raw?path=");
  });
});
