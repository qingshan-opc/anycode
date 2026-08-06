export type TextAttachment = {
  filename: string;
  content: string;
};

/** Paste longer than this becomes a reference card instead of inline textarea text. */
export const PASTE_AS_CARD_MIN_CHARS = 4000;
export const MAX_TEXT_FILES = 10;
/** Raw upload cap; extracted text is truncated server-side, big files routed to disk pointers. */
export const MAX_TEXT_FILE_BYTES = 4 * 1024 * 1024;

/** Extensions the composer base64-encodes instead of reading as text (binary formats). */
export const BINARY_FILE_EXTENSIONS = ["pdf", "xlsx", "docx", "pptx"] as const;

export function isBinaryTextAttachment(filename: string): boolean {
  const ext = filename.toLowerCase().split(".").pop() ?? "";
  return (BINARY_FILE_EXTENSIONS as readonly string[]).includes(ext);
}

export function plainTextFromPasteEvent(event: ClipboardEvent): string {
  return event.clipboardData?.getData("text/plain") ?? "";
}

export function shouldPasteAsTextCard(text: string): boolean {
  return text.length > PASTE_AS_CARD_MIN_CHARS;
}

export function utf8ByteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

export function makePastedTextAttachment(
  content: string,
  existingFilenames: string[],
): TextAttachment {
  let n = existingFilenames.length + 1;
  let filename = `paste-${n}.txt`;
  const used = new Set(existingFilenames);
  while (used.has(filename)) {
    n += 1;
    filename = `paste-${n}.txt`;
  }
  return { filename, content };
}

export function textPayloadsForApi(
  files: TextAttachment[],
): { filename: string; content: string }[] {
  return files.map(({ filename, content }) => ({ filename, content }));
}

export function formatTextAttachmentMeta(content: string, locale: string): string {
  const n = content.length;
  try {
    return new Intl.NumberFormat(locale === "zh" ? "zh-CN" : "en-US").format(n);
  } catch {
    return String(n);
  }
}
