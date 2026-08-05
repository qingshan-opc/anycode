import { describe, expect, it } from "vitest";
import { countUnseenArtifacts } from "@/lib/artifactBadge";

describe("countUnseenArtifacts", () => {
  it("returns 0 without a watermark (pre-existing artifacts are not unread)", () => {
    const artifacts = [{ updated_at: "2026-08-05T10:00:00Z" }];
    expect(countUnseenArtifacts(artifacts, null)).toBe(0);
    expect(countUnseenArtifacts(artifacts, undefined)).toBe(0);
  });

  it("counts only artifacts updated after the watermark", () => {
    const artifacts = [
      { updated_at: "2026-08-05T09:00:00Z" }, // before → seen
      { updated_at: "2026-08-05T11:00:00Z" }, // after → unread
      { updated_at: "2026-08-05T12:00:00Z" }, // after → unread
      { updated_at: null }, // unknown → ignored
      {}, // missing → ignored
    ];
    expect(countUnseenArtifacts(artifacts, "2026-08-05T10:00:00Z")).toBe(2);
  });

  it("returns 0 for a malformed watermark or malformed timestamps", () => {
    expect(countUnseenArtifacts([{ updated_at: "2026-08-05T11:00:00Z" }], "junk")).toBe(0);
    expect(
      countUnseenArtifacts([{ updated_at: "junk" }], "2026-08-05T10:00:00Z"),
    ).toBe(0);
  });

  it("exact watermark boundary is seen (not unread)", () => {
    const artifacts = [{ updated_at: "2026-08-05T10:00:00.000Z" }];
    expect(countUnseenArtifacts(artifacts, "2026-08-05T10:00:00.000Z")).toBe(0);
  });
});
