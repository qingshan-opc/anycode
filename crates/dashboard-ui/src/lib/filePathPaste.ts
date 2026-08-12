/**
 * Detect pasted text that is a list of local file paths (one per line).
 * Returns the paths, or null when the text is ordinary prose (default paste
 * should proceed). Absolute POSIX paths, `~/…`, and Windows drive paths are
 * recognized; relative paths are intentionally NOT (too easy to false-positive
 * on sentences).
 */
export function pastedFilePaths(text: string): string[] | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const lines = trimmed
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  if (lines.length === 0 || lines.length > 10) return null;
  const PATH_RE = /^(\/\S(?:.*)|~\/.*|[A-Za-z]:[\\/].*)$/;
  if (!lines.every((l) => PATH_RE.test(l))) return null;
  // A lone "/" or "~/" is a directory, not a file paste — let the caller
  // decide, but "/" alone carries no intent; treat as prose.
  if (lines.every((l) => l === "/" || l === "~" || l === "~/")) return null;
  return lines;
}
