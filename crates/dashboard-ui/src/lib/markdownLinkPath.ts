import { isAbsolutePath, resolveDeliverableAbsPath } from "@/lib/deliverablePath";

/** True for http(s), mailto, tel, data, blob — open via browser/external. */
export function isExternalHref(href: string): boolean {
  const h = href.trim();
  if (!h) return false;
  return /^(https?:|mailto:|tel:|data:|blob:)/i.test(h);
}

/**
 * True when href looks like a local filesystem path (relative or absolute),
 * not an in-app hash/route or external URL.
 */
export function isLocalPathHref(href: string): boolean {
  const h = href.trim();
  if (!h || isExternalHref(h)) return false;
  if (h.startsWith("#") || h.startsWith("?") || h.startsWith("javascript:")) {
    return false;
  }
  if (h.startsWith("file:")) return true;
  if (isAbsolutePath(h)) return true;
  // Relative project paths: ./foo, ../bar, crates/x, docs/a.md
  if (h.startsWith("./") || h.startsWith("../")) return true;
  if (!h.includes("://") && /^[\w.@+-]+(?:\/[\w.@+-]+)+/.test(h)) return true;
  if (!h.includes("://") && /\.(md|rs|ts|tsx|js|jsx|json|toml|yaml|yml|txt|py|go|css|html)$/i.test(h)) {
    return true;
  }
  return false;
}

/** Strip file:// prefix and resolve against project root when relative. */
export function resolveMarkdownLocalPath(
  href: string,
  projectRoot?: string | null,
): string {
  let path = href.trim();
  if (path.startsWith("file://")) {
    try {
      path = decodeURIComponent(path.slice("file://".length));
    } catch {
      path = path.slice("file://".length);
    }
    // file:///Users/... → /Users/...
    if (/^\/[A-Za-z]:\//.test(path)) {
      path = path.slice(1);
    }
  }
  return resolveDeliverableAbsPath(path, projectRoot);
}
