import {
  imageFilesFromPasteEvent,
  pastedImageFromBase64,
} from "@/lib/clipboardImage";
import {
  formatTextAttachmentMeta,
  makePastedTextAttachment,
  MAX_TEXT_FILE_BYTES,
  MAX_TEXT_FILES,
  plainTextFromPasteEvent,
  shouldPasteAsTextCard,
  utf8ByteLength,
  type TextAttachment,
} from "@/lib/composerTextAttachment";
import {
  MAX_IMAGE_BYTES,
  MAX_VISION_IMAGES,
  visionAttachmentFromBase64,
  type VisionAttachment,
} from "@/lib/composerVision";
import { pastedFilePaths } from "@/lib/filePathPaste";
import { readFilePaths } from "@/api/client/files";
import { readApplePasteboard } from "@/lib/desktopShell";

const IMAGE_FILE_EXT = /\.(png|jpe?g|gif|webp|bmp|tiff?|heic|avif|svg)$/i;

export type ComposerPasteResult =
  | { kind: "ignored" }
  | { kind: "images"; images: VisionAttachment[]; error?: string }
  | { kind: "text-card"; file: TextAttachment; hint: string }
  | { kind: "text-cards"; files: TextAttachment[]; hint: string }
  | { kind: "error"; error: string };

type PasteHandlers = {
  /** Native vision OR OCR capability — may accept image paste. */
  canAttachImages: boolean;
  attachedImageCount: number;
  attachedTextFiles: TextAttachment[];
  locale: string;
  t: (key: string) => string;
  ingestImageFiles: (files: File[]) => Promise<void>;
  /**
   * Local file paths pasted as text — the composer supplies the backend
   * read + attachment conversion. When omitted, path-looking text falls
   * through to the default paste.
   */
  ingestFilePaths?: (paths: string[]) => Promise<ComposerPasteResult>;
};

/**
 * Shared composer paste: images → vision attachments; text >4000 chars → reference card.
 * Must call preventDefault synchronously before any await for long-text / browser images.
 */
export async function handleComposerPasteEvent(
  event: ClipboardEvent,
  handlers: PasteHandlers,
): Promise<ComposerPasteResult> {
  const {
    canAttachImages,
    attachedImageCount,
    attachedTextFiles,
    locale,
    t,
    ingestImageFiles,
  } = handlers;

  const imageFiles = imageFilesFromPasteEvent(event);
  if (imageFiles.length > 0) {
    event.preventDefault();
    await ingestImageFiles(imageFiles);
    return { kind: "ignored" };
  }

  // Finder ⌘C of a file/dir: Chrome/WKWebView surface it as DataTransferItem
  // kind "file" (non-image).
  const { files: pastedFiles, dirNames: pastedDirs } = nonImageFilesFromPasteEvent(event);
  if (pastedFiles.length > 0 || pastedDirs.length > 0) {
    event.preventDefault();
    // Desktop: prefer the native pasteboard's real paths — binaries and
    // directories become path-reference cards the agent can open with its
    // file/office tools. Browser paste carries only bytes, so there we read
    // text content client-side and report the rest as skipped.
    if (handlers.ingestFilePaths) {
      const pbPaths = await pasteboardFilePaths();
      if (pbPaths.length > 0) return handlers.ingestFilePaths(pbPaths);
    }
    return ingestPastedFiles(pastedFiles, pastedDirs, attachedTextFiles, t);
  }

  // Long text must be gated before any await — otherwise default paste wins.
  const pastedText = plainTextFromPasteEvent(event);

  // 粘贴本地文件路径(单行或多行):读文件内容并作为参考附件。
  const paths = pastedFilePaths(pastedText);
  if (paths && handlers.ingestFilePaths) {
    event.preventDefault();
    return handlers.ingestFilePaths(paths);
  }

  if (shouldPasteAsTextCard(pastedText)) {
    event.preventDefault();
    if (attachedTextFiles.length >= MAX_TEXT_FILES) {
      return { kind: "error", error: t("conversations.attachmentPasteLimit") };
    }
    if (utf8ByteLength(pastedText) > MAX_TEXT_FILE_BYTES) {
      return {
        kind: "error",
        error: t("conversations.attachmentTextTooLarge").replace("{name}", "paste.txt"),
      };
    }
    const file = makePastedTextAttachment(
      pastedText,
      attachedTextFiles.map((f) => f.filename),
    );
    return {
      kind: "text-card",
      file,
      hint: t("conversations.attachmentPasteAsCard").replace(
        "{n}",
        formatTextAttachmentMeta(pastedText, locale),
      ),
    };
  }

  // Short/empty text: optionally pull image from Apple pasteboard (desktop).
  if (pastedText.trim().length > 0) {
    return { kind: "ignored" };
  }

  event.preventDefault();
  const pbItems = await readApplePasteboard();
  const imageItem = pbItems.find((item) => item.kind === "image" && item.data_base64);

  // Finder ⌘C in the desktop app: WKWebView may expose nothing on the web
  // event, but the native pasteboard carries file URLs with real paths — route
  // them through the backend file reader (dirs become path-reference cards).
  // Image-file copies keep the vision path below.
  const filePaths = pbItems
    .filter((item) => item.kind === "file_url" && item.text?.trim())
    .map((item) => item.text!.trim());
  if (filePaths.length > 0 && handlers.ingestFilePaths) {
    const imageOnly =
      Boolean(imageItem?.data_base64) && filePaths.every((p) => IMAGE_FILE_EXT.test(p));
    if (!imageOnly) {
      return handlers.ingestFilePaths(filePaths);
    }
  }

  if (!imageItem?.data_base64) {
    return { kind: "ignored" };
  }
  if (!canAttachImages) {
    return { kind: "error", error: t("conversations.attachmentVisionDisabled") };
  }
  if (attachedImageCount >= MAX_VISION_IMAGES) {
    return { kind: "ignored" };
  }
  const payload = pastedImageFromBase64(imageItem.mime_type, imageItem.data_base64);
  const approxBytes = Math.floor((payload.data_base64.length * 3) / 4);
  if (approxBytes > MAX_IMAGE_BYTES) {
    return {
      kind: "error",
      error: t("conversations.attachmentImageTooLarge").replace("{name}", "image"),
    };
  }
  return {
    kind: "images",
    images: [visionAttachmentFromBase64(payload.mime_type, payload.data_base64)],
  };
}

/**
 * File paths from the native pasteboard (desktop only; [] in the browser).
 * Finder ⌘C of files/dirs lands here as `file_url` items with real paths.
 */
async function pasteboardFilePaths(): Promise<string[]> {
  const pbItems = await readApplePasteboard();
  return pbItems
    .filter((item) => item.kind === "file_url" && item.text?.trim())
    .map((item) => item.text!.trim());
}

/**
 * Non-image files carried by the paste event (Finder file/dir ⌘C). Directory
 * items are separated out — their File reads as empty bytes and must not
 * become a blank card.
 */
function nonImageFilesFromPasteEvent(event: ClipboardEvent): {
  files: File[];
  dirNames: string[];
} {
  const files: File[] = [];
  const dirNames: string[] = [];
  const items = event.clipboardData?.items;
  if (!items) return { files, dirNames };
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    if (!item || item.kind !== "file" || item.type.startsWith("image/")) continue;
    const entry = item.webkitGetAsEntry?.();
    if (entry?.isDirectory) {
      dirNames.push(entry.name || item.getAsFile()?.name || "folder");
      continue;
    }
    const file = item.getAsFile();
    if (file) files.push(file);
  }
  return { files, dirNames };
}

/**
 * Read pasted non-image File objects into reference cards. Paste carries
 * bytes but no absolute path, so directories / binaries / oversize files are
 * collected into a skip note (same `name(kind)` style as the path reader).
 */
async function ingestPastedFiles(
  files: File[],
  dirNames: string[],
  attachedTextFiles: TextAttachment[],
  t: (key: string) => string,
): Promise<ComposerPasteResult> {
  const cards: TextAttachment[] = [];
  const skipped: string[] = dirNames.map((n) => `${n}(dir)`);
  const taken = new Set(attachedTextFiles.map((f) => f.filename));
  for (const file of files) {
    if (taken.size >= MAX_TEXT_FILES) {
      skipped.push(`${file.name}(limit)`);
      continue;
    }
    if (file.size > MAX_TEXT_FILE_BYTES) {
      skipped.push(`${file.name}(too-large)`);
      continue;
    }
    let content: string;
    try {
      content = await file.text();
    } catch {
      skipped.push(`${file.name || "file"}(unreadable)`);
      continue;
    }
    if (content.slice(0, 8192).includes("\0")) {
      skipped.push(`${file.name}(binary)`);
      continue;
    }
    let name = file.name || "paste.txt";
    while (taken.has(name)) name = `_${name}`;
    taken.add(name);
    cards.push({ filename: name, content });
  }
  if (cards.length === 0) {
    return {
      kind: "error",
      error: t("conversations.attachmentPathUnreadable").replace(
        "{names}",
        skipped.join(", "),
      ),
    };
  }
  let hint = t("conversations.attachmentPathLoaded").replace("{n}", String(cards.length));
  if (skipped.length > 0) {
    hint = `${hint} · ${t("conversations.attachmentPathSkipped").replace("{names}", skipped.join(", "))}`;
  }
  return { kind: "text-cards", files: cards, hint };
}

/**
 * Default `ingestFilePaths` implementation: read the pasted local paths via
 * the dashboard API and turn readable text files into reference attachments.
 * Directories and binaries (docx/pdf/zip …) become path-reference cards — the
 * agent reads them with its file/office tools; only missing paths are skipped.
 */
export async function ingestPastedFilePaths(
  paths: string[],
  t: (key: string) => string,
): Promise<ComposerPasteResult> {
  try {
    const { files } = await readFilePaths(paths);
    const texts = files.filter((f) => f.kind === "text" && f.content != null);
    const pathCards = files
      .filter((f) => f.kind === "dir" || f.kind === "binary")
      .map((f) => ({
        filename: f.kind === "dir" ? `${f.name}/` : f.name,
        content: f.kind === "dir" ? `[directory] ${f.path}` : `[file] ${f.path}`,
      }));
    const cards = [
      ...texts.map((f) => ({ filename: f.name, content: f.content ?? "" })),
      ...pathCards,
    ];
    if (cards.length === 0) {
      return {
        kind: "error",
        error: t("conversations.attachmentPathUnreadable").replace(
          "{names}",
          files.map((f) => f.name).join(", "),
        ),
      };
    }
    const skipped = files.filter((f) => f.kind !== "text" && f.kind !== "dir" && f.kind !== "binary");
    const notes: string[] = [];
    if (skipped.length > 0) {
      notes.push(
        t("conversations.attachmentPathSkipped").replace(
          "{names}",
          skipped.map((f) => `${f.name}(${f.kind})`).join(", "),
        ),
      );
    }
    if (texts.some((f) => f.truncated)) {
      notes.push(t("conversations.attachmentPathTruncated"));
    }
    let hint = t("conversations.attachmentPathLoaded").replace(
      "{n}",
      String(cards.length),
    );
    if (notes.length > 0) hint = `${hint} · ${notes.join(" · ")}`;
    return {
      kind: "text-cards",
      files: cards,
      hint,
    };
  } catch (e) {
    return { kind: "error", error: e instanceof Error ? e.message : String(e) };
  }
}
