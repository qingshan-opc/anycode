import { post } from "@/api/http";

export type DiagramKind = "mermaid" | "mindmap" | "math";

export type DiagramSourceResponse = {
  id: string;
  session_id: string | null;
  kind: DiagramKind;
  source: string;
  title: string | null;
  created_at: string;
};

/** kind+source → persisted id, so re-renders don't re-POST. */
const persistCache = new Map<string, string>();

/**
 * Persist a rendered diagram and return its stable share id (`dgm_…`).
 * Returns null on any failure — sharing is best-effort and must never break
 * inline rendering. Ids are content-hashed server-side, so the cache key is
 * just kind+source.
 */
export async function persistDiagram(
  kind: DiagramKind,
  source: string,
  opts: { sessionId?: string | null; title?: string | null } = {},
): Promise<string | null> {
  const trimmed = source.trim();
  if (!trimmed) return null;
  const key = `${kind}\n${trimmed}`;
  const cached = persistCache.get(key);
  if (cached) return cached;
  try {
    const res = await post<{ id: string; url: string }>("/api/diagrams", {
      kind,
      source: trimmed,
      session_id: opts.sessionId ?? null,
      title: opts.title ?? null,
    });
    persistCache.set(key, res.id);
    return res.id;
  } catch {
    return null;
  }
}

/** Absolute share URL for the standalone /diagram/{id} page. */
export function diagramShareUrl(id: string): string {
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  return `${origin}/diagram/${encodeURIComponent(id)}`;
}

/** Fetch a persisted diagram for the standalone share page (unauthenticated). */
export async function fetchDiagramSource(id: string): Promise<DiagramSourceResponse> {
  const res = await fetch(`/diagram/${encodeURIComponent(id)}/source`);
  if (!res.ok) {
    throw new Error(`${res.status} ${res.statusText}`);
  }
  return (await res.json()) as DiagramSourceResponse;
}
