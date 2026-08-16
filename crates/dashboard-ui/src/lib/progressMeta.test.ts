import { describe, expect, it } from "vitest";
import {
  formatDeliveryPreflight,
  isInternalProgressDiagnostic,
  isStaticProgressStatusLine,
  progressPhase,
  progressSummary,
  shouldRenderAssistantAsStatusLine,
} from "./progressMeta";
import type { TranscriptBlock } from "@/api/types";

describe("delivery preflight formatting", () => {
  it("never renders delivery_preflight marker", () => {
    const raw =
      "[delivery_preflight] family=office_delivery skill=anycode-ppt brand=fde-editorial scenario=anycode-ppt artifacts=[deck_html_slides:html] gates=2";
    expect(formatDeliveryPreflight(raw)).toBeNull();
  });

  it("maps compile work_stage to intent phase", () => {
    const block = {
      id: "1",
      block_type: "progress_update",
      body: "",
      meta: { phase: "gate", work_stage: "compile", summary: "x" },
    } as TranscriptBlock;
    expect(progressPhase(block)).toBe("intent");
  });

  it("progressSummary suppresses preflight", () => {
    const block = {
      id: "1",
      block_type: "progress_update",
      body: "",
      meta: {
        summary:
          "[delivery_preflight] family=office_delivery skill=anycode-docx brand=fde-editorial scenario=work-report artifacts=[report_docx:docx] gates=3",
      },
    } as TranscriptBlock;
    expect(progressSummary(block)).toBe("");
  });

  it("progressSummary suppresses gate and skill markers", () => {
    expect(
      progressSummary({
        id: "g",
        block_type: "progress_update",
        body: "[gate_plan_created] family=office gates=2",
        meta: {},
      } as TranscriptBlock),
    ).toBe("");
    expect(
      progressSummary({
        id: "s",
        block_type: "progress_update",
        body: "",
        meta: { summary: "[skill_resolved] anycode-ppt" },
      } as TranscriptBlock),
    ).toBe("");
  });

  it("isInternalProgressDiagnostic matches compile markers", () => {
    expect(isInternalProgressDiagnostic("[delivery_preflight] family=general")).toBe(true);
    expect(isInternalProgressDiagnostic("正在检查测试")).toBe(false);
  });

  it("marks progress_update and narration status as static (no fold chevron)", () => {
    expect(
      isStaticProgressStatusLine({
        id: "p",
        block_type: "progress_update",
        body: "收尾前最后确认",
        meta: { summary: "收尾前最后确认" },
      } as TranscriptBlock),
    ).toBe(true);
    expect(
      isStaticProgressStatusLine({
        id: "n",
        block_type: "assistant_message",
        body: "planning",
        meta: { narration: true, message_role: "status" },
      } as TranscriptBlock),
    ).toBe(true);
    expect(
      isStaticProgressStatusLine({
        id: "t",
        block_type: "system_notice",
        body: "thinking",
        meta: { source: "thinking_delta" },
      } as TranscriptBlock),
    ).toBe(true);
    expect(
      isStaticProgressStatusLine({
        id: "i",
        block_type: "system_notice",
        body: "继续实施 D1",
        meta: { source: "intermediate_assistant" },
      } as TranscriptBlock),
    ).toBe(true);
    expect(
      isStaticProgressStatusLine({
        id: "live",
        block_type: "assistant_message",
        body: "planning",
        meta: { live: true },
      } as TranscriptBlock),
    ).toBe(true);
  });

  it("routes live and superseded assistant text to status lines", () => {
    const liveBlock = {
      id: "live",
      block_type: "assistant_message",
      body: "继续实施",
      meta: { live: true },
    } as TranscriptBlock;
    expect(
      shouldRenderAssistantAsStatusLine(liveBlock, {
        itemIndex: 0,
        finalAssistantIndex: 0,
      }),
    ).toBe(true);
    const settledMidTurn = {
      id: "mid",
      block_type: "assistant_message",
      body: "earlier plan",
      meta: {},
    } as TranscriptBlock;
    expect(
      shouldRenderAssistantAsStatusLine(settledMidTurn, {
        itemIndex: 0,
        finalAssistantIndex: 2,
      }),
    ).toBe(true);
    const finalReply = {
      id: "final",
      block_type: "assistant_message",
      body: "done",
      meta: {},
    } as TranscriptBlock;
    expect(
      shouldRenderAssistantAsStatusLine(finalReply, {
        itemIndex: 2,
        finalAssistantIndex: 2,
      }),
    ).toBe(false);
  });
});
