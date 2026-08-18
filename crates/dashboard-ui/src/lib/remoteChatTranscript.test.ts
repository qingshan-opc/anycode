import { describe, expect, it } from "vitest";
import { cloudEventsToLines } from "@/lib/remoteChatTranscript";

describe("cloudEventsToLines", () => {
  it("merges assistant deltas after a user prompt", () => {
    const lines = cloudEventsToLines([
      { seq: 1, kind: "user_prompt", payload: { text: "hi" }, created_at: "" },
      { seq: 2, kind: "assistant_delta", payload: { text: "hel" }, created_at: "" },
      { seq: 3, kind: "assistant_delta", payload: { text: "lo" }, created_at: "" },
      { seq: 4, kind: "assistant_done", payload: {}, created_at: "" },
    ]);
    expect(lines).toEqual([
      { id: "u-1", role: "user", text: "hi" },
      { id: "a-3", role: "assistant", text: "hello" },
    ]);
  });
});
