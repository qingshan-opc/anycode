import type { CloudRemoteEvent } from "@/api/client/accountCloud";

export type RemoteTranscriptLine = {
  id: string;
  role: "user" | "assistant" | "tool" | "error";
  text: string;
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
