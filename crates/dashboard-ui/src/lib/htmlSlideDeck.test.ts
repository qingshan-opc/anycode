import { describe, expect, it } from "vitest";
import type { FsEntry } from "@/api/types/workbench";
import {
  initialSlideIndex,
  isHtmlPath,
  isSlideHtmlName,
  parentDir,
  deckIndexPath,
  resolveDeckSlides,
  slidesFromFsEntries,
  slidesFromManifest,
} from "./htmlSlideDeck";

function file(path: string): FsEntry {
  const name = path.split("/").pop() ?? path;
  return { name, path, kind: "file" };
}

describe("htmlSlideDeck", () => {
  it("detects html and numbered slides", () => {
    expect(isHtmlPath("slides/01-cover.html")).toBe(true);
    expect(isSlideHtmlName("01-cover.html")).toBe(true);
    expect(isSlideHtmlName("index.html")).toBe(false);
    expect(parentDir("slides/index.html")).toBe("slides");
  });

  it("orders numbered siblings and skips index.html", () => {
    const entries = [
      file("slides/index.html"),
      file("slides/12-closing.html"),
      file("slides/01-cover.html"),
      file("slides/slide_manifest.json"),
    ];
    expect(slidesFromFsEntries(entries)).toEqual([
      "slides/01-cover.html",
      "slides/12-closing.html",
    ]);
  });

  it("prefers slide_manifest source_html", () => {
    expect(
      slidesFromManifest("slides", {
        slides: [{ source_html: "01-cover.html" }, { source_html: "02-section.html" }],
      }),
    ).toEqual(["slides/01-cover.html", "slides/02-section.html"]);
  });

  it("falls back to slides/ when index.html sits beside the folder", () => {
    const slides = resolveDeckSlides({
      path: "index.html",
      entries: [file("index.html"), { name: "slides", path: "slides", kind: "dir" }],
      nestedEntries: [file("slides/01-cover.html"), file("slides/02-section.html")],
    });
    expect(slides).toEqual(["slides/01-cover.html", "slides/02-section.html"]);
  });

  it("starts at the selected slide, or first page for index.html", () => {
    const slides = ["slides/01-cover.html", "slides/02-section.html"];
    expect(initialSlideIndex(slides, "slides/02-section.html")).toBe(1);
    expect(initialSlideIndex(slides, "slides/index.html")).toBe(0);
  });

  it("opens the deck index.html instead of a single slide", () => {
    expect(
      deckIndexPath({
        selectedPath: "slides/01-cover.html",
        entries: [file("slides/index.html"), file("slides/01-cover.html")],
      }),
    ).toBe("slides/index.html");
    expect(
      deckIndexPath({
        selectedPath: "slides/index.html",
        entries: [file("slides/index.html")],
      }),
    ).toBe("slides/index.html");
    expect(
      deckIndexPath({
        selectedPath: "slides/01-cover.html",
        entries: [],
      }),
    ).toBe("slides/index.html");
  });
});
