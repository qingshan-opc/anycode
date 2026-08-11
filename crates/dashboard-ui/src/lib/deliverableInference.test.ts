import { describe, expect, it } from "vitest";
import { collectInlineDeliverables } from "./deliverableInference";
import type { TranscriptBlock } from "@/api/types";

describe("collectInlineDeliverables", () => {
  it("renders cards from explicit ANYCODE_ARTIFACT markers in assistant text", () => {
    const assistant: TranscriptBlock = {
      id: "a1",
      block_type: "assistant_message",
      at: "2026-01-01T00:00:00Z",
      title: "",
      body: '报告已生成。\nANYCODE_ARTIFACT:{"path":"/proj/sales-report.xlsx","kind":"spreadsheet","title":"销售汇总"}',
    };
    const byBlock = collectInlineDeliverables(
      [{ kind: "block", block: assistant }],
      "project-1",
    );
    const cards = byBlock.get("a1") ?? [];
    expect(cards).toHaveLength(1);
    expect(cards[0]?.path).toBe("/proj/sales-report.xlsx");
    expect(cards[0]?.kind).toBe("spreadsheet");
    expect(cards[0]?.title).toBe("销售汇总");
    expect(cards[0]?.projectId).toBe("project-1");
  });

  it("does not infer cards from FileWrite tool steps", () => {
    // 申报制：写文件（多为代码）不产生交付物卡片。
    const assistant: TranscriptBlock = {
      id: "a1",
      block_type: "assistant_message",
      at: "2026-01-01T00:00:00Z",
      title: "",
      body: "已写入 mindmap-anycode-complex.md。",
    };
    const byBlock = collectInlineDeliverables(
      [
        {
          kind: "tool_cluster",
          id: "tools-1",
          steps: [
            {
              key: "t1",
              call: {
                id: "c1",
                block_type: "tool_call",
                at: "",
                title: "FileWrite started",
                body: "",
                meta: { name: "FileWrite" },
              },
              result: {
                id: "r1",
                block_type: "tool_result",
                at: "",
                title: "FileWrite finished",
                body: "",
                meta: {
                  name: "FileWrite",
                  path: "/proj/mindmap-anycode-complex.md",
                },
              },
            },
          ],
          processMessageCount: 0,
          processSnippets: [],
        },
        { kind: "block", block: assistant },
      ],
      "project-1",
    );
    expect(byBlock.get("a1") ?? []).toHaveLength(0);
  });

  it("does not infer cards from prose path mentions", () => {
    const assistant: TranscriptBlock = {
      id: "a1",
      block_type: "assistant_message",
      at: "2026-01-01T00:00:00Z",
      title: "",
      body: "已写入 sales-report.csv，详情见 `docs/architecture.md`。",
    };
    const byBlock = collectInlineDeliverables(
      [{ kind: "block", block: assistant }],
      "project-1",
    );
    expect(byBlock.get("a1") ?? []).toHaveLength(0);
  });

  it("skips markers already covered by persisted deliverable blocks", () => {
    const deliverable: TranscriptBlock = {
      id: "d1",
      block_type: "deliverable",
      at: "2026-01-01T00:00:00Z",
      title: "deck.pptx",
      body: "/proj/deck.pptx",
      meta: { path: "/proj/deck.pptx", kind: "presentation" },
    };
    const assistant: TranscriptBlock = {
      id: "a1",
      block_type: "assistant_message",
      at: "2026-01-01T00:00:00Z",
      title: "",
      body: 'ANYCODE_ARTIFACT:{"path":"/proj/deck.pptx","kind":"presentation"}',
    };
    const byBlock = collectInlineDeliverables(
      [
        { kind: "block", block: deliverable },
        { kind: "block", block: assistant },
      ],
      "project-1",
    );
    expect(byBlock.get("a1") ?? []).toHaveLength(0);
  });
});
