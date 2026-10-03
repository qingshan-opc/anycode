import { expect, test } from "@playwright/test";

test("new conversation without projects keeps directory registration reachable", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("anycode-dashboard-locale", "en");
  });
  await page.route("**/api/projects?*", (route) =>
    route.fulfill({ json: { projects: [], total: 0, limit: 200, offset: 0 } }),
  );
  await page.route("**/api/sessions?*", (route) =>
    route.fulfill({ json: { sessions: [] } }),
  );
  await page.goto("/conversations?new=1");
  await expect(page.locator(".conv-thread-composer .dw-project-picker__trigger")).toBeVisible();
  await expect(page).toHaveURL(/new=(true|1)/);
  await page.getByRole("button", { name: "New session", exact: true }).click();
  await expect(page).toHaveURL(/new=(true|1)/);

  await page.locator(".conv-thread-composer .dw-project-picker__trigger").click();
  await page.locator(".dw-project-picker__item--action").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog.locator("input[required]").fill("/synthetic/first-project");
  await expect(dialog.locator("button[type=submit]")).toBeEnabled();
  await expect(page).toHaveURL(/new=(true|1)/);
  // No submission: this proves UI reachability without creating a directory,
  // registering a project, or dispatching an Agent/provider request.
});
