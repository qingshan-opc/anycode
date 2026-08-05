import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { WorkbenchTab } from "@/api/types/workbench";
import { countUnseenArtifacts } from "@/lib/artifactBadge";
import { useWorkbenchSidebarState } from "./useWorkbenchSidebarState";

/**
 * Unread count for the Artifacts tab: final artifacts updated after the
 * user's last-seen watermark. Shares its queryKey with ArtifactsPanel's
 * default (final-only) list, so the SSE deliverable invalidation keeps both
 * in sync with no extra fetching.
 */
export function useArtifactsBadge(sessionId: string | null): number {
  const { lastSeen } = useWorkbenchSidebarState();
  const query = useQuery({
    queryKey: ["session-artifacts", sessionId, "final"],
    queryFn: () => api.sessionArtifacts(sessionId!, { finalOnly: true, limit: 100 }),
    enabled: Boolean(sessionId),
    staleTime: 5_000,
  });
  return countUnseenArtifacts(query.data?.artifacts ?? [], lastSeen.artifacts);
}

/**
 * Composes every panel's badge hook in a fixed order (rules-of-hooks) —
 * same pattern as useWorkbenchAutoOpen. Returns counts keyed by tab id;
 * zero/absent entries are not rendered.
 */
export function useWorkbenchBadges(
  sessionId: string | null,
): Partial<Record<WorkbenchTab, number>> {
  const artifacts = useArtifactsBadge(sessionId);
  return { artifacts };
}
