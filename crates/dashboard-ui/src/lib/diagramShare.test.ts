import { beforeEach, describe, expect, it, vi } from "vitest";

const postMock = vi.fn();
vi.mock("@/api/http", () => ({
  post: (...args: unknown[]) => postMock(...args),
}));

import { diagramShareUrl, persistDiagram } from "./diagramShare";

describe("diagramShare", () => {
  beforeEach(() => {
    postMock.mockReset();
  });

  it("persists once per kind+source and reuses the stable id", async () => {
    postMock.mockResolvedValue({ id: "dgm_abc", url: "/diagram/dgm_abc" });
    const first = await persistDiagram("mermaid", "graph TD; A-->B", {
      sessionId: "sess-1",
    });
    const again = await persistDiagram("mermaid", "graph TD; A-->B");
    expect(first).toBe("dgm_abc");
    expect(again).toBe("dgm_abc");
    expect(postMock).toHaveBeenCalledTimes(1);
    expect(postMock).toHaveBeenCalledWith("/api/diagrams", {
      kind: "mermaid",
      source: "graph TD; A-->B",
      session_id: "sess-1",
      title: null,
    });
  });

  it("distinguishes kinds for identical source text", async () => {
    postMock.mockResolvedValue({ id: "dgm_x", url: "/diagram/dgm_x" });
    await persistDiagram("mindmap", "# Root");
    await persistDiagram("math", "# Root");
    expect(postMock).toHaveBeenCalledTimes(2);
  });

  it("fails soft (null, no throw) when persistence errors", async () => {
    postMock.mockRejectedValue(new Error("500 boom"));
    await expect(persistDiagram("mermaid", "graph TD; X-->Y")).resolves.toBeNull();
  });

  it("builds share URLs on the diagram route", () => {
    expect(diagramShareUrl("dgm_abc")).toMatch(/\/diagram\/dgm_abc$/);
  });
});
