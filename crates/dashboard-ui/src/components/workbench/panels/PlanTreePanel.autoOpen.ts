import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { WorkbenchTab } from "@/api/types/workbench";
import { PlanAutoOpenTracker } from "@/lib/workbenchAutoOpen";

export type PlanAutoOpenCtx = {
  sessionId: string | null;
  openTab: (tab: WorkbenchTab) => void;
};

/**
 * New plan revision → open the Plan panel for human review. The initial
 * hydrate never opens (see PlanAutoOpenTracker). The plan-tree query shares
 * its cache with PlanTreePanel via the same queryKey.
 */
export function usePlanAutoOpen({ sessionId, openTab }: PlanAutoOpenCtx): void {
  const trackerRef = useRef(new PlanAutoOpenTracker());

  useEffect(() => {
    trackerRef.current.reset();
  }, [sessionId]);

  const planTreeQuery = useQuery({
    queryKey: ["session-plan-tree", sessionId],
    queryFn: () => api.sessionPlanTree(sessionId!),
    enabled: Boolean(sessionId),
    staleTime: 5_000,
  });

  useEffect(() => {
    if (!sessionId) return;
    const updatedAt = planTreeQuery.data?.updated_at ?? null;
    const rootsCount = planTreeQuery.data?.tree?.roots?.length ?? 0;
    if (trackerRef.current.ingest(updatedAt, rootsCount)) {
      openTab("plan");
    }
  }, [sessionId, planTreeQuery.data, openTab]);
}
