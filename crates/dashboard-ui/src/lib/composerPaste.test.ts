import { describe, expect, it, vi } from "vitest";
import { handleComposerPasteEvent, ingestPastedFilePaths } from "./composerPaste";
import { PASTE_AS_CARD_MIN_CHARS } from "./composerTextAttachment";

vi.mock("@/lib/desktopShell", () => ({
  readApplePasteboard: vi.fn(async () => []),
}));

vi.mock("@/api/client/files", () => ({
  readFilePaths: vi.fn(async () => ({ files: [] })),
}));

import { readApplePasteboard } from "@/lib/desktopShell";
import { readFilePaths } from "@/api/client/files";

function pasteEvent(text: string): ClipboardEvent {
  return {
    preventDefault: vi.fn(),
    clipboardData: {
      getData: (type: string) => (type === "text/plain" ? text : ""),
      items: [],
    },
  } as unknown as ClipboardEvent;
}

function pasteEventWithItems(items: Partial<DataTransferItem>[]): ClipboardEvent {
  return {
    preventDefault: vi.fn(),
    clipboardData: {
      getData: () => "",
      items,
    },
  } as unknown as ClipboardEvent;
}

function fileItem(file: File, entry?: { isDirectory: boolean; name: string }) {
  return {
    kind: "file",
    type: file.type,
    getAsFile: () => file,
    webkitGetAsEntry: () => entry ?? null,
  } as Partial<DataTransferItem>;
}

const baseHandlers = {
  canAttachImages: true,
  attachedImageCount: 0,
  attachedTextFiles: [],
  locale: "zh",
  // Keep placeholders so .replace("{names}" …) substitutions stay visible.
  t: (key: string) => `${key} {names} {n} {name}`,
  ingestImageFiles: vi.fn(async () => {}),
};

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

describe("Finder file/dir copy (event-carried files)", () => {
  it("reads a pasted text file into a reference card", async () => {
    const file = new File(["hello 文件"], "notes.md", { type: "text/markdown" });
    const event = pasteEventWithItems([fileItem(file)]);
    const result = await handleComposerPasteEvent(event, baseHandlers);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(result.kind).toBe("text-cards");
    if (result.kind === "text-cards") {
      expect(result.files).toEqual([{ filename: "notes.md", content: "hello 文件" }]);
    }
  });

  it("reports a pasted directory as skipped instead of a blank card", async () => {
    const dirFile = new File([], "assets", { type: "" });
    const event = pasteEventWithItems([
      fileItem(dirFile, { isDirectory: true, name: "assets" }),
    ]);
    const result = await handleComposerPasteEvent(event, baseHandlers);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(result.kind).toBe("error");
    if (result.kind === "error") {
      expect(result.error).toContain("assets(dir)");
    }
  });

  it("mixes a readable file with a skipped binary", async () => {
    const text = new File(["body"], "a.txt", { type: "text/plain" });
    const bin = new File([new Uint8Array([0, 159, 146])], "b.bin", {
      type: "application/octet-stream",
    });
    const event = pasteEventWithItems([fileItem(text), fileItem(bin)]);
    const result = await handleComposerPasteEvent(event, baseHandlers);
    expect(result.kind).toBe("text-cards");
    if (result.kind === "text-cards") {
      expect(result.files.map((f) => f.filename)).toEqual(["a.txt"]);
      expect(result.hint).toContain("b.bin(binary)");
    }
  });

  it("dedupes filenames against already-attached files", async () => {
    const file = new File(["x"], "a.txt", { type: "text/plain" });
    const event = pasteEventWithItems([fileItem(file)]);
    const result = await handleComposerPasteEvent(event, {
      ...baseHandlers,
      attachedTextFiles: [{ filename: "a.txt", content: "old" }],
    });
    expect(result.kind).toBe("text-cards");
    if (result.kind === "text-cards") {
      expect(result.files[0]!.filename).not.toBe("a.txt");
    }
  });

  it("on desktop prefers pasteboard paths so binaries get path cards", async () => {
    const bin = new File([new Uint8Array([0, 159, 146])], "汇报版本.docx", {
      type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    });
    vi.mocked(readApplePasteboard).mockResolvedValueOnce([
      { kind: "file_url", text: "/Users/me/Desktop/汇报版本.docx" },
    ]);
    const ingestFilePaths = vi.fn(async () => ({
      kind: "text-cards" as const,
      files: [{ filename: "汇报版本.docx", content: "[file] /Users/me/Desktop/汇报版本.docx" }],
      hint: "h",
    }));
    const event = pasteEventWithItems([fileItem(bin)]);
    const result = await handleComposerPasteEvent(event, {
      ...baseHandlers,
      ingestFilePaths,
    });
    expect(event.preventDefault).toHaveBeenCalled();
    expect(ingestFilePaths).toHaveBeenCalledWith(["/Users/me/Desktop/汇报版本.docx"]);
    expect(result.kind).toBe("text-cards");
  });
});

describe("desktop pasteboard file_url fallback", () => {
  it("routes file URLs to ingestFilePaths when the web event is empty", async () => {
    vi.mocked(readApplePasteboard).mockResolvedValueOnce([
      { kind: "file_url", text: "/tmp/a.md" },
      { kind: "file_url", text: "/tmp/docs" },
    ]);
    const ingestFilePaths = vi.fn(async () => ({
      kind: "text-cards" as const,
      files: [{ filename: "a.md", content: "x" }],
      hint: "h",
    }));
    const event = pasteEvent("");
    const result = await handleComposerPasteEvent(event, {
      ...baseHandlers,
      ingestFilePaths,
    });
    expect(event.preventDefault).toHaveBeenCalled();
    expect(ingestFilePaths).toHaveBeenCalledWith(["/tmp/a.md", "/tmp/docs"]);
    expect(result.kind).toBe("text-cards");
  });

  it("keeps image-file copies on the vision path", async () => {
    vi.mocked(readApplePasteboard).mockResolvedValueOnce([
      { kind: "file_url", text: "/tmp/shot.png" },
      { kind: "image", mime_type: "image/png", data_base64: "aGk=" },
    ]);
    const ingestFilePaths = vi.fn();
    const event = pasteEvent("");
    const result = await handleComposerPasteEvent(event, {
      ...baseHandlers,
      ingestFilePaths,
    });
    expect(ingestFilePaths).not.toHaveBeenCalled();
    expect(result.kind).toBe("images");
  });

  it("routes image files without pasteboard image data to the file reader", async () => {
    vi.mocked(readApplePasteboard).mockResolvedValueOnce([
      { kind: "file_url", text: "/tmp/shot.png" },
    ]);
    const ingestFilePaths = vi.fn(async () => ({
      kind: "error" as const,
      error: "unreadable",
    }));
    const event = pasteEvent("");
    const result = await handleComposerPasteEvent(event, {
      ...baseHandlers,
      ingestFilePaths,
    });
    expect(ingestFilePaths).toHaveBeenCalledWith(["/tmp/shot.png"]);
    expect(result.kind).toBe("error");
  });
});

describe("ingestPastedFilePaths", () => {
  it("turns directories and binaries into path-reference cards", async () => {
    vi.mocked(readFilePaths).mockResolvedValueOnce({
      files: [
        { path: "/tmp/docs", name: "docs", kind: "dir", size_bytes: 0, truncated: false },
        {
          path: "/tmp/a.md",
          name: "a.md",
          kind: "text",
          content: "hi",
          size_bytes: 2,
          truncated: false,
        },
        { path: "/tmp/b.docx", name: "b.docx", kind: "binary", size_bytes: 3, truncated: false },
      ],
    });
    const result = await ingestPastedFilePaths(["/tmp/docs", "/tmp/a.md", "/tmp/b.docx"], baseHandlers.t);
    expect(result.kind).toBe("text-cards");
    if (result.kind === "text-cards") {
      expect(result.files).toEqual([
        { filename: "a.md", content: "hi" },
        { filename: "docs/", content: "[directory] /tmp/docs" },
        { filename: "b.docx", content: "[file] /tmp/b.docx" },
      ]);
      expect(result.hint).not.toContain("attachmentPathSkipped ·");
    }
  });

  it("errors when nothing readable was pasted", async () => {
    vi.mocked(readFilePaths).mockResolvedValueOnce({
      files: [
        { path: "/tmp/gone.txt", name: "gone.txt", kind: "missing", size_bytes: 0, truncated: false },
      ],
    });
    const result = await ingestPastedFilePaths(["/tmp/gone.txt"], baseHandlers.t);
    expect(result.kind).toBe("error");
  });
});
