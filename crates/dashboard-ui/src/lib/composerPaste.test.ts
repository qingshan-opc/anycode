import { describe, expect, it, vi } from "vitest";
import { handleComposerPasteEvent } from "./composerPaste";
import { PASTE_AS_CARD_MIN_CHARS } from "./composerTextAttachment";

function pasteEvent(text: string): ClipboardEvent {
  return {
    preventDefault: vi.fn(),
    clipboardData: {
      getData: (type: string) => (type === "text/plain" ? text : ""),
      items: [],
    },
  } as unknown as ClipboardEvent;
}

describe("handleComposerPasteEvent", () => {
  it("turns long text into a reference card and prevents default before await", async () => {
    const event = pasteEvent("x".repeat(PASTE_AS_CARD_MIN_CHARS + 1));
    const result = await handleComposerPasteEvent(event, {
      canAttachImages: true,
      attachedImageCount: 0,
      attachedTextFiles: [],
      locale: "zh",
      t: (key) => key,
      ingestImageFiles: vi.fn(),
    });
    expect(event.preventDefault).toHaveBeenCalled();
    expect(result.kind).toBe("text-card");
    if (result.kind === "text-card") {
      expect(result.file.filename).toBe("paste-1.txt");
      expect(result.file.content.length).toBe(PASTE_AS_CARD_MIN_CHARS + 1);
    }
  });

  it("ignores short text so default paste can proceed", async () => {
    const event = pasteEvent("hello");
    const result = await handleComposerPasteEvent(event, {
      canAttachImages: true,
      attachedImageCount: 0,
      attachedTextFiles: [],
      locale: "zh",
      t: (key) => key,
      ingestImageFiles: vi.fn(),
    });
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(result).toEqual({ kind: "ignored" });
  });
});

describe("file path paste", () => {
  it("delegates path-looking text to ingestFilePaths with preventDefault", async () => {
    const event = pasteEvent("/tmp/a.md\n~/b.md");
    const ingestFilePaths = vi.fn(async () => ({
      kind: "text-cards" as const,
      files: [{ filename: "a.md", content: "x" }],
      hint: "h",
    }));
    const result = await handleComposerPasteEvent(event, {
      canAttachImages: true,
      attachedImageCount: 0,
      attachedTextFiles: [],
      locale: "zh",
      t: (key) => key,
      ingestImageFiles: vi.fn(),
      ingestFilePaths,
    });
    expect(event.preventDefault).toHaveBeenCalled();
    expect(ingestFilePaths).toHaveBeenCalledWith(["/tmp/a.md", "~/b.md"]);
    expect(result.kind).toBe("text-cards");
  });

  it("falls through to default paste for paths when no handler is wired", async () => {
    const event = pasteEvent("/tmp/a.md");
    const result = await handleComposerPasteEvent(event, {
      canAttachImages: true,
      attachedImageCount: 0,
      attachedTextFiles: [],
      locale: "zh",
      t: (key) => key,
      ingestImageFiles: vi.fn(),
    });
    expect(result.kind).toBe("ignored");
  });
});
