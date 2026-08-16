import { describe, expect, it } from "vitest";
import type { TranscriptBlock } from "@/api/types";
import {
  countLogicalToolSteps,
  groupTurnReplies,
  mergeFinalAssistantBlocks,
  toolResultFailed,
  toolStepFailed,
} from "@/lib/transcriptGrouping";
import type { ToolStep } from "@/lib/transcriptGrouping";

function block(
  id: string,
  blockType: string,
  extra?: Partial<TranscriptBlock>,
): TranscriptBlock {
  return {
    id,
    block_type: blockType,
    at: "2026-01-01T00:00:00Z",
    title: "Bash",
    body: "",
    ...extra,
  };
}

describe("groupTurnReplies", () => {
  it("keeps agent narration visible before its tool evidence", () => {
    const replies = [
      block("a1", "assistant_message", {
        body: "planning",
        meta: { narration: true, message_role: "status" },
      }),
      block("t1", "tool_call", { meta: { tool_key: "1:1", phase: "start" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1", phase: "end" } }),
      block("t3", "tool_call", { meta: { tool_key: "1:2", phase: "start" } }),
      block("f1", "assistant_message", { body: "done" }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual(["block", "tool_cluster", "block"]);
    if (grouped[0]?.kind === "block") {
      expect(grouped[0].block.body).toBe("planning");
    }
    if (grouped[1]?.kind === "tool_cluster") {
      expect(grouped[1].processSnippets).not.toContain("planning");
    }
    if (grouped[3]?.kind === "block") {
      expect(grouped[3].block.body).toBe("done");
    }
  });

  it("keeps all intermediate_assistant narration on the timeline before tools", () => {
    const replies = [
      block("n0", "system_notice", {
        meta: { source: "intermediate_assistant" },
        body: "oldest step",
      }),
      block("n1", "system_notice", {
        meta: { source: "intermediate_assistant" },
        body: "checking env",
      }),
      block("t1", "tool_call", { meta: { tool_key: "1:1" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1" } }),
      block("mid", "system_notice", {
        meta: { source: "intermediate_assistant" },
        body: "now edit files",
      }),
      block("t3", "tool_call", { meta: { tool_key: "1:2" } }),
      block("delta", "system_notice", {
        meta: { source: "thinking_delta" },
        body: "internal thought",
      }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual([
      "block",
      "block",
      "tool_cluster",
      "block",
      "tool_cluster",
      "block",
    ]);
    if (grouped[0]?.kind === "block") {
      expect(grouped[0].block.body).toBe("oldest step");
    }
    if (grouped[1]?.kind === "block") {
      expect(grouped[1].block.body).toBe("checking env");
    }
    if (grouped[3]?.kind === "block") {
      expect(grouped[3].block.body).toBe("now edit files");
    }
    if (grouped[5]?.kind === "block") {
      expect(grouped[5].block.body).toBe("internal thought");
      expect(grouped[5].block.meta?.source).toBe("thinking_delta");
    }
  });

  it("interleaves thinking_delta prose with tool pills (Cursor waterfall)", () => {
    const replies = [
      block("th1", "system_notice", {
        meta: { source: "thinking_delta", turn: 1 },
        body: "先读一下现状",
      }),
      block("t1", "tool_call", { meta: { tool_key: "1:1", name: "Read" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1", name: "Read" } }),
      block("th2", "system_notice", {
        meta: { source: "thinking_delta", turn: 2 },
        body: "接下来改状态机",
      }),
      block("t3", "tool_call", { meta: { tool_key: "1:2", name: "Edit" } }),
      block("t4", "tool_result", { meta: { tool_key: "1:2", name: "Edit" } }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual([
      "block",
      "tool_cluster",
      "block",
      "tool_cluster",
    ]);
    if (grouped[0]?.kind === "block") {
      expect(grouped[0].block.body).toBe("先读一下现状");
    }
    if (grouped[2]?.kind === "block") {
      expect(grouped[2].block.body).toBe("接下来改状态机");
    }
    if (grouped[1]?.kind === "tool_cluster") {
      expect(grouped[1].processSnippets).toEqual([]);
    }
  });

  it("merges tool clusters separated only by system notices", () => {
    const replies = [
      block("t1", "tool_call", { meta: { tool_key: "1:1", name: "Grep" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1", name: "Grep" } }),
      block("n1", "system_notice", { body: "scanning", meta: {} }),
      block("t3", "tool_call", { meta: { tool_key: "1:2", name: "Glob" } }),
      block("t4", "tool_result", { meta: { tool_key: "1:2", name: "Glob" } }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.filter((item) => item.kind === "tool_cluster")).toHaveLength(1);
    const cluster = grouped.find((item) => item.kind === "tool_cluster");
    if (cluster?.kind === "tool_cluster") {
      expect(cluster.steps.length).toBe(2);
      expect(cluster.processSnippets).toContain("scanning");
    }
  });

  it("keeps live assistant bubbles even when body is still empty", () => {
    const replies = [
      block("live", "assistant_message", { body: "", meta: { live: true } }),
      block("t1", "tool_call", { meta: { tool_key: "1:1" } }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped[0]?.kind).toBe("block");
    if (grouped[0]?.kind === "block") {
      expect(grouped[0].block.meta?.live).toBe(true);
    }
  });

  it("interleaves mid-turn assistant text with following tools", () => {
    const replies = [
      block("a1", "assistant_message", { body: "先读取 HTML" }),
      block("t1", "tool_call", { meta: { tool_key: "1:1", name: "Read" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1", name: "Read" } }),
      block("a2", "assistant_message", { body: "开始修改 Hero" }),
      block("t3", "tool_call", { meta: { tool_key: "1:2", name: "Edit" } }),
      block("t4", "tool_result", { meta: { tool_key: "1:2", name: "Edit" } }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual([
      "block",
      "tool_cluster",
      "block",
      "tool_cluster",
    ]);
    if (grouped[0]?.kind === "block") {
      expect(grouped[0].block.body).toBe("先读取 HTML");
    }
    if (grouped[2]?.kind === "block") {
      expect(grouped[2].block.body).toBe("开始修改 Hero");
    }
  });
});

describe("groupTurnReplies subagent groups", () => {
  const sa = (taskId: string, agentType = "explore", parentTaskId: string | null = null) => ({
    subagent: { task_id: taskId, agent_type: agentType, parent_task_id: parentTaskId },
  });

  it("groups consecutive same-task blocks into a collapsible subagent group", () => {
    const replies = [
      block("u1", "assistant_message", { body: "let me delegate" }),
      block("h1", "system_notice", {
        meta: { source: "subagent_start", ...sa("task-a") },
      }),
      block("t1", "tool_call", {
        meta: { tool_key: "u3:satask-a:1:1", phase: "start", ...sa("task-a") },
      }),
      block("t2", "tool_result", {
        meta: { tool_key: "u3:satask-a:1:1", phase: "end", ...sa("task-a") },
      }),
      block("d1", "system_notice", {
        meta: { source: "subagent_done", status: "completed", ...sa("task-a") },
      }),
      block("f1", "assistant_message", { body: "final answer" }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual([
      "block",
      "subagent_group",
      "block",
    ]);
    const group = grouped[1];
    if (group?.kind === "subagent_group") {
      expect(group.taskId).toBe("task-a");
      expect(group.agentType).toBe("explore");
      expect(group.status).toBe("completed");
      expect(group.toolCount).toBe(1);
      // header / done markers fold into chrome, only tool cluster remains
      expect(group.items.map((item) => item.kind)).toEqual(["tool_cluster"]);
    }
  });

  it("aggregates interleaved children into one card each at first sight", () => {
    const replies = [
      block("a1", "tool_call", {
        meta: { tool_key: "u3:satask-a:1:1", phase: "start", ...sa("task-a") },
      }),
      block("b1", "tool_call", {
        meta: { tool_key: "u3:satask-b:1:1", phase: "start", ...sa("task-b", "plan") },
      }),
      block("a2", "tool_result", {
        meta: { tool_key: "u3:satask-a:1:1", phase: "end", ...sa("task-a") },
      }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual([
      "subagent_group",
      "subagent_group",
    ]);
    const [ga, gb] = grouped;
    if (ga?.kind === "subagent_group") {
      // interleaved child events still aggregate per child (call+result pair)
      expect(ga.taskId).toBe("task-a");
      expect(ga.status).toBeNull();
      expect(ga.toolCount).toBe(1);
    }
    if (gb?.kind === "subagent_group") {
      expect(gb.agentType).toBe("plan");
      expect(gb.status).toBeNull();
    }
  });

  it("nests grandchild blocks as their own subagent group (depth 2)", () => {
    const replies = [
      block("h1", "system_notice", {
        meta: { source: "subagent_start", ...sa("task-a") },
      }),
      block("a1", "tool_call", {
        meta: { tool_key: "u3:satask-a:1:1", phase: "start", ...sa("task-a") },
      }),
      block("a2", "tool_result", {
        meta: { tool_key: "u3:satask-a:1:1", phase: "end", ...sa("task-a") },
      }),
      // grandchild: tagged with its own task id inside task-a's stream
      block("b1", "tool_call", {
        meta: { tool_key: "u3:satask-b:1:1", phase: "start", ...sa("task-b", "plan", "task-a") },
      }),
      block("b2", "tool_result", {
        meta: { tool_key: "u3:satask-b:1:1", phase: "end", ...sa("task-b", "plan", "task-a") },
      }),
      block("d1", "system_notice", {
        meta: { source: "subagent_done", status: "completed", ...sa("task-a") },
      }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual(["subagent_group"]);
    const ga = grouped[0];
    if (ga?.kind === "subagent_group") {
      expect(ga.taskId).toBe("task-a");
      expect(ga.status).toBe("completed");
      // own activity flattened (tool cluster), grandchild grouped separately
      expect(ga.items.map((item) => item.kind)).toEqual([
        "tool_cluster",
        "subagent_group",
      ]);
      const gb = ga.items[1];
      if (gb?.kind === "subagent_group") {
        expect(gb.taskId).toBe("task-b");
        expect(gb.agentType).toBe("plan");
        expect(gb.toolCount).toBe(1);
      }
    }
  });

  it("does not re-group a group's own blocks into itself (no infinite recursion)", () => {
    const replies = [
      block("a1", "assistant_message", {
        body: "child narration",
        meta: { narration: true, message_role: "status", ...sa("task-a") },
      }),
      block("a2", "assistant_message", { body: "child final", ...sa("task-a") }),
    ];
    const grouped = groupTurnReplies(replies);
    const ga = grouped[0];
    if (ga?.kind === "subagent_group") {
      // own blocks stay flat inside the group — no nested group with same id
      expect(
        ga.items.every(
          (item) => item.kind !== "subagent_group" || item.taskId !== "task-a",
        ),
      ).toBe(true);
    }
  });

  it("leaves blocks without subagent tags untouched", () => {
    const replies = [
      block("t1", "tool_call", { meta: { tool_key: "1:1", phase: "start" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1", phase: "end" } }),
    ];
    const grouped = groupTurnReplies(replies);
    expect(grouped.map((item) => item.kind)).toEqual(["tool_cluster"]);
  });
});

describe("mergeFinalAssistantBlocks", () => {  it("merges multiple assistant messages into one bubble", () => {
    const merged = mergeFinalAssistantBlocks([
      block("a1", "assistant_message", { body: "part 1" }),
      block("a2", "assistant_message", { body: "part 2", meta: { live: true } }),
    ]);
    expect(merged).toHaveLength(1);
    expect(merged[0]?.body).toBe("part 1\n\npart 2");
    expect(merged[0]?.meta?.live).toBe(true);
  });
});

describe("countLogicalToolSteps", () => {
  it("counts by tool_key not raw blocks", () => {
    const tools = [
      block("t1", "tool_call", { meta: { tool_key: "1:1" } }),
      block("t2", "tool_result", { meta: { tool_key: "1:1" } }),
      block("t3", "tool_call", { meta: { tool_key: "1:2" } }),
      block("t4", "tool_result", { meta: { tool_key: "1:2" } }),
    ];
    expect(countLogicalToolSteps(tools)).toBe(2);
  });
});

function toolStep(title: string, body: string, withResult = true): ToolStep {
  const call = { title, body: "", at: "2026-01-01T00:00:00Z", id: "call", block_type: "tool_call" } as TranscriptBlock;
  const result = {
    title,
    body,
    at: "2026-01-01T00:00:00Z",
    id: "result",
    block_type: "tool_result",
  } as TranscriptBlock;
  return { key: "1:1", call, ...(withResult ? { result } : {}) };
}

describe("toolResultFailed", () => {
  it("trusts server-generated failed title", () => {
    expect(toolResultFailed("Bash failed", "everything is fine")).toBe(true);
  });

  it("trusts server-generated finished title even when body mentions error", () => {
    expect(toolResultFailed("Bash finished", "no error detected")).toBe(false);
    expect(toolResultFailed("Bash finished", "exit_code: 1")).toBe(false);
  });

  it("reads exit_code from structured JSON", () => {
    expect(toolResultFailed("Bash", JSON.stringify({ exit_code: 0 }))).toBe(false);
    expect(toolResultFailed("Bash", JSON.stringify({ exit_code: 1 }))).toBe(true);
  });

  it("reads success boolean from structured JSON", () => {
    expect(toolResultFailed("Bash", JSON.stringify({ success: true }))).toBe(false);
    expect(toolResultFailed("Bash", JSON.stringify({ success: false }))).toBe(true);
  });

  it("reads error string from structured JSON", () => {
    expect(toolResultFailed("Bash", JSON.stringify({ error: "boom" }))).toBe(true);
    expect(toolResultFailed("Bash", JSON.stringify({ error: "" }))).toBe(false);
  });

  it("treats known success payloads as success even when words appear", () => {
    expect(toolResultFailed("FileRead", JSON.stringify({ content: "no error here" }))).toBe(false);
    expect(toolResultFailed("Glob", JSON.stringify({ filenames: ["failed.txt"] }))).toBe(false);
    expect(toolResultFailed("Grep", JSON.stringify({ matches: ["error"] }))).toBe(false);
    expect(toolResultFailed("WebSearch", JSON.stringify({ raw: "denied" }))).toBe(false);
  });

  it("recognizes deterministic text failure prefixes", () => {
    expect(toolResultFailed("Bash", "Command failed: cargo build")).toBe(true);
    expect(toolResultFailed("Bash", "Command timed out after 120s")).toBe(true);
    expect(toolResultFailed("FileRead", "File not found: src/main.rs")).toBe(true);
    expect(toolResultFailed("FileRead", "Not a file: /dev/null")).toBe(true);
    expect(toolResultFailed("Bash", "Permission denied")).toBe(true);
    expect(toolResultFailed("Grep", "rg failed")).toBe(true);
    expect(toolResultFailed("FileWrite", "Serialization error: missing field file_path")).toBe(true);
    expect(toolResultFailed("Glob", "path escapes sandbox")).toBe(true);
    expect(toolResultFailed("Skill", "skill exited with code 1: pandoc not installed")).toBe(true);
  });

  it("recognizes HTTP and Other error prefixes", () => {
    expect(toolResultFailed("WebFetch", "HTTP 404 Not Found")).toBe(true);
    expect(toolResultFailed("WebFetch", "HTTP 500 Internal Server Error")).toBe(true);
    expect(toolResultFailed("WebSearch", "Other error: ddg: error sending request")).toBe(true);
  });

  it("does not treat arbitrary body words as failure", () => {
    expect(toolResultFailed("Bash", "3 failed, 2 passed, 1 error")).toBe(false);
    expect(toolResultFailed("Read", "## Error Handling\nDocs about errors.")).toBe(false);
    expect(toolResultFailed("Bash", "denied: some package name")).toBe(false);
  });
});

describe("toolStepFailed", () => {
  it("uses result body when present", () => {
    expect(toolStepFailed(toolStep("Bash", "Command failed: cargo build"))).toBe(true);
    expect(toolStepFailed(toolStep("Bash", "3 failed, 2 passed"))).toBe(false);
  });

  it("falls back to call body when no result yet", () => {
    expect(toolStepFailed(toolStep("Bash", "still running", false))).toBe(false);
  });
});
