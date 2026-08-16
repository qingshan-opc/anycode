import { apiUrl } from "@/api/http";
import { isAbsolutePath } from "@/lib/deliverablePath";

function posix(path: string): string {
  return path.trim().replace(/\\/g, "/");
}

/** Project-relative path for `/fs/raw/{*rel}` (empty if it cannot be made relative). */
export function projectFsRawRelPath(path: string, projectRoot?: string | null): string | null {
  const normalized = posix(path);
  if (!normalized) return "";
  const root = posix(projectRoot ?? "").replace(/\/+$/, "");
  if (root && (normalized === root || normalized.startsWith(`${root}/`))) {
    return normalized.slice(root.length).replace(/^\/+/, "");
  }
  if (isAbsolutePath(normalized)) {
    return null;
  }
  return normalized.replace(/^\.\//, "").replace(/^\/+/, "");
}

/** Absolute URL to stream a project file (img/video/pdf/html/download). */
export function projectFsRawUrl(
  projectId: string,
  path: string,
  projectRoot?: string | null,
): string {
  const rel = projectFsRawRelPath(path, projectRoot);
  if (rel == null) {
    return apiUrl(
      `/api/projects/${encodeURIComponent(projectId)}/fs/raw?path=${encodeURIComponent(path.trim())}`,
    );
  }
  if (!rel) {
    return apiUrl(`/api/projects/${encodeURIComponent(projectId)}/fs/raw?path=`);
  }
  const segments = rel
    .split("/")
    .filter(Boolean)
    .map((part) => encodeURIComponent(part))
    .join("/");
  return apiUrl(`/api/projects/${encodeURIComponent(projectId)}/fs/raw/${segments}`);
}
