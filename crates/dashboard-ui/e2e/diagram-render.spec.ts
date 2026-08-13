import { expect, test } from "@playwright/test";

// P1.8 leftover: manual render verification for mermaid `xychart-beta`.
// The composer/chat path and the public share page both feed the fence body
// straight into mermaid.render, so rendering on the share page proves the
// pipeline end to end (API -> SPA route -> mermaid 11).
test.describe("diagram share rendering", () => {
  test("xychart-beta renders to SVG on the share page", async ({
    page,
    request,
  }) => {
    const source = [
      "xychart-beta",
      '    title "Weekly tokens"',
      "    x-axis [mon, tue, wed, thu, fri]",
      "    bar [2, 4, 3, 5, 6]",
      "    line [1, 3, 2, 4, 5]",
    ].join("\n");
    const res = await request.post("/api/diagrams", {
      data: { kind: "mermaid", source, title: "xychart smoke" },
    });
    expect(res.ok()).toBeTruthy();
    const { id } = (await res.json()) as { id: string };

    await page.goto(`/diagram/${id}`);
    const canvas = page.locator(".diagram-share__canvas");
    await expect(canvas.locator("svg")).toBeVisible({ timeout: 15_000 });
    // Render failure falls back to a <pre> source dump; assert we are not in
    // the fallback state.
    await expect(page.locator("pre.dw-transcript-code")).toHaveCount(0);
  });
});
