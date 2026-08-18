import { apiFetch } from "../api";

export type RemoteMineDevice = {
  id: string;
  device_name: string;
  platform: string;
  last_seen_at: string;
  online: boolean;
  this_device: boolean;
};

export type CloudRemoteConversation = {
  id: string;
  home_device_id: string;
  local_session_id?: string | null;
  title: string;
  project_name?: string | null;
  status: string;
  last_event_seq: number;
  created_at: string;
  updated_at: string;
};

export type CloudRemoteEvent = {
  seq: number;
  kind: string;
  payload: unknown;
  created_at: string;
};

export type RemoteTranscriptLine = {
  id: string;
  role: "user" | "assistant" | "tool" | "error";
  text: string;
};

export const remoteChatApi = {
  listMineDevices: () =>
    apiFetch<{ devices: RemoteMineDevice[] }>("/api/v1/devices/mine"),

  listConversations: (homeDeviceId: string) =>
    apiFetch<{ conversations: CloudRemoteConversation[] }>(
      `/api/v1/remote-chat/conversations?home_device_id=${encodeURIComponent(homeDeviceId)}`,
    ),

  getConversation: (conversationId: string, after = 0) =>
    apiFetch<{ conversation: CloudRemoteConversation; events: CloudRemoteEvent[] }>(
      `/api/v1/remote-chat/conversations/${encodeURIComponent(conversationId)}?after=${after}`,
    ),

  postPrompt: (body: {
    home_device_id: string;
    prompt: string;
    conversation_id?: string;
    title?: string;
  }) =>
    apiFetch<{ ok: boolean; conversation: CloudRemoteConversation }>(
      "/api/v1/remote-chat/prompt",
      { method: "POST", body: JSON.stringify(body) },
    ),

  postCancel: (conversationId: string) =>
    apiFetch<{ ok: boolean }>("/api/v1/remote-chat/cancel", {
      method: "POST",
      body: JSON.stringify({ conversation_id: conversationId }),
    }),
};

function payloadText(payload: unknown): string {
  if (!payload || typeof payload !== "object") return "";
  const p = payload as Record<string, unknown>;
  if (typeof p.text === "string") return p.text;
  if (p.payload && typeof p.payload === "object") {
    const inner = p.payload as Record<string, unknown>;
    if (typeof inner.text === "string") return inner.text;
  }
  return "";
}

/** Flatten cloud turn events into visible transcript lines. */
export function cloudEventsToLines(events: CloudRemoteEvent[]): RemoteTranscriptLine[] {
  const lines: RemoteTranscriptLine[] = [];
  let assistantBuf = "";
  let assistantSeq: number | null = null;

  const flushAssistant = () => {
    if (assistantBuf.trim() && assistantSeq !== null) {
      lines.push({
        id: `a-${assistantSeq}`,
        role: "assistant",
        text: assistantBuf,
      });
    }
    assistantBuf = "";
    assistantSeq = null;
  };

  for (const event of events) {
    const kind = event.kind;
    const text = payloadText(event.payload);
    if (kind === "user_prompt" || kind === "user_message") {
      flushAssistant();
      if (text.trim()) {
        lines.push({ id: `u-${event.seq}`, role: "user", text });
      }
      continue;
    }
    if (kind === "session_error") {
      flushAssistant();
      lines.push({
        id: `e-${event.seq}`,
        role: "error",
        text: text.trim() || "error",
      });
      continue;
    }
    if (kind === "tool_start" || kind === "tool_result") {
      flushAssistant();
      const name =
        event.payload && typeof event.payload === "object"
          ? String(
              (event.payload as { tool_name?: string; tool_key?: string }).tool_name ??
                (event.payload as { tool_key?: string }).tool_key ??
                "tool",
            )
          : "tool";
      lines.push({
        id: `t-${event.seq}`,
        role: "tool",
        text: name,
      });
      continue;
    }
    if (kind === "assistant_delta") {
      assistantSeq = event.seq;
      assistantBuf += text;
      continue;
    }
    if (kind === "assistant_done" || kind === "turn_done") {
      if (text.trim()) assistantBuf = text;
      flushAssistant();
    }
  }
  flushAssistant();
  return lines;
}

export function relativeTime(iso: string, locale: string): string {
  const ms = Date.parse(iso);
  if (!Number.isFinite(ms)) return "";
  const diff = Date.now() - ms;
  const rtf = new Intl.RelativeTimeFormat(locale.startsWith("zh") ? "zh-CN" : "en", {
    numeric: "auto",
  });
  const min = Math.round(diff / 60_000);
  if (Math.abs(min) < 60) return rtf.format(-min, "minute");
  const hr = Math.round(min / 60);
  if (Math.abs(hr) < 48) return rtf.format(-hr, "hour");
  return rtf.format(-Math.round(hr / 24), "day");
}
