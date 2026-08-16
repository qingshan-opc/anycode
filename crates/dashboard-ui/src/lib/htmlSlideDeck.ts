import type { FsEntry } from "@/api/types/workbench";
import { basename, extension } from "@/lib/pathUtils";

export type SlideManifest = {
  slides?: Array<{ source_html?: string }>;
};

export function isHtmlPath(path: string): boolean {
  const ext = extension(path);
  return ext === "html" || ext === "htm";
}

export function parentDir(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const slash = normalized.lastIndexOf("/");
  if (slash <= 0) return "";
  return normalized.slice(0, slash);
}

export function isDeckIndexName(name: string): boolean {
  return name.toLowerCase() === "index.html" || name.toLowerCase() === "index.htm";
}

/** Numbered slide files like `01-cover.html`, not the deck index. */
export function isSlideHtmlName(name: string): boolean {
  const lower = name.toLowerCase();
  if (!lower.endsWith(".html") && !lower.endsWith(".htm")) return false;
  if (isDeckIndexName(name)) return false;
  return /^\d+/.test(name);
}

export function joinDir(dir: string, name: string): string {
  const file = name.replace(/\\/g, "/").replace(/^\.\//, "");
  if (!dir) return file;
  if (file.includes("/")) return file;
  return `${dir.replace(/\\/g, "/").replace(/\/+$/, "")}/${file}`;
}

export function slidesFromManifest(baseDir: string, manifest: SlideManifest | null | undefined): string[] {
  const slides = (manifest?.slides ?? [])
    .map((slide) => slide.source_html?.trim())
    .filter((value): value is string => Boolean(value));
  return slides.map((source) => joinDir(baseDir, source));
}

export function slidesFromFsEntries(entries: FsEntry[]): string[] {
  return entries
    .filter((entry) => entry.kind === "file" && isSlideHtmlName(entry.name))
    .map((entry) => entry.path)
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
}

/** Prefer numbered siblings; if viewing index.html with none, caller should list `slides/`. */
export function resolveDeckSlides(opts: {
  path: string;
  entries: FsEntry[];
  nestedEntries?: FsEntry[];
  manifest?: SlideManifest | null;
}): string[] {
  const dir = parentDir(opts.path);
  const fromManifest = slidesFromManifest(dir, opts.manifest);
  if (fromManifest.length > 0) return fromManifest;
  const siblings = slidesFromFsEntries(opts.entries);
  if (siblings.length > 0) return siblings;
  if (isDeckIndexName(basename(opts.path)) && opts.nestedEntries) {
    return slidesFromFsEntries(opts.nestedEntries);
  }
  return [];
}

export function initialSlideIndex(slides: string[], selectedPath: string): number {
  if (slides.length === 0) return 0;
  const normalized = selectedPath.replace(/\\/g, "/");
  const idx = slides.findIndex((slide) => slide.replace(/\\/g, "/") === normalized);
  if (idx >= 0) return idx;
  return 0;
}

/**
 * Browser-open target for a deck: the `index.html` viewer, not a single slide.
 * Falls back to the selected file when no index exists.
 */
export function deckIndexPath(opts: {
  selectedPath: string;
  entries: FsEntry[];
  nestedEntries?: FsEntry[];
}): string {
  const selected = opts.selectedPath.replace(/\\/g, "/");
  if (isDeckIndexName(basename(selected))) return selected;

  const inDir = opts.entries.find(
    (entry) => entry.kind === "file" && isDeckIndexName(entry.name),
  );
  if (inDir) return inDir.path;

  const inNested = (opts.nestedEntries ?? []).find(
    (entry) => entry.kind === "file" && isDeckIndexName(entry.name),
  );
  if (inNested) return inNested.path;

  const dir = parentDir(selected);
  if (dir) return `${dir}/index.html`;
  return selected;
}
