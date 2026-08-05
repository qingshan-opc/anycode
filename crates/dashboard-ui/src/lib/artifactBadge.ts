import type { ArtifactRecord } from "@/api/types";

/**
 * Count artifacts updated after the user's "seen" watermark — the unread
 * badge for the Artifacts workbench tab. No watermark → 0 (a freshly
 * introduced badge must not light up for pre-existing deliverables).
 */
export function countUnseenArtifacts(
  artifacts: readonly Pick<ArtifactRecord, "updated_at">[],
  seenAt: string | null | undefined,
): number {
  if (!seenAt) return 0;
  const seenMs = Date.parse(seenAt);
  if (Number.isNaN(seenMs)) return 0;
  let count = 0;
  for (const a of artifacts) {
    if (!a.updated_at) continue;
    const updatedMs = Date.parse(a.updated_at);
    if (!Number.isNaN(updatedMs) && updatedMs > seenMs) count += 1;
  }
  return count;
}
