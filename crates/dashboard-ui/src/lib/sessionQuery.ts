import type { QueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { SessionWithProject } from "@/api/types";

export type SessionsListCache = { sessions?: SessionWithProject[] };

/** Drop a session from React Query `all-sessions*` list payloads. */
export function omitSessionFromSessionsCache(
  prev: SessionsListCache | undefined,
  sessionId: string,
): SessionsListCache | undefined {
  if (!prev?.sessions) return prev;
  const sessions = prev.sessions.filter((session) => session.id !== sessionId);
  if (sessions.length === prev.sessions.length) return prev;
  return { ...prev, sessions };
}

export const SESSION_QUERY_GC_MS = 30 * 60_000;
export const TRANSCRIPT_STALE_RUNNING_MS = 3_000;

export function transcriptStaleTime(
  isRunning: boolean,
  chatStreamLive = false,
  streamLive = false,
): number {
  if (chatStreamLive || streamLive) {
    return Number.POSITIVE_INFINITY;
  }
  return isRunning ? TRANSCRIPT_STALE_RUNNING_MS : Number.POSITIVE_INFINITY;
}

export function transcriptQueryOptions(
  sessionId: string,
  isRunning = false,
  chatStreamLive = false,
  streamLive = false,
) {
  return {
    queryKey: ["session-transcript", sessionId] as const,
    // API builds from `chat_turn_events` when present; log-tail is legacy fallback only.
    queryFn: () => api.sessionTranscript(sessionId),
    staleTime: transcriptStaleTime(isRunning, chatStreamLive, streamLive),
    gcTime: SESSION_QUERY_GC_MS,
  };
}

export function sessionArtifactsQueryOptions(sessionId: string, isRunning = false) {
  return {
    queryKey: ["session-artifacts", sessionId] as const,
    queryFn: () => api.sessionArtifacts(sessionId, { finalOnly: true }),
    staleTime: isRunning ? TRANSCRIPT_STALE_RUNNING_MS : Number.POSITIVE_INFINITY,
    gcTime: SESSION_QUERY_GC_MS,
  };
}

export function prefetchSessionConversation(
  queryClient: QueryClient,
  sessionId: string,
  isRunning = false,
) {
  void queryClient.prefetchQuery(transcriptQueryOptions(sessionId, isRunning));
  void queryClient.prefetchQuery(sessionArtifactsQueryOptions(sessionId, isRunning));
}
