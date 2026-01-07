import { test, expect } from "playwright/test";
import type { Page } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

async function ensureArchivedExpanded(page: Page) {
  const toggle = page.getByRole("button", { name: "Archived" });
  await toggle.waitFor({ timeout: 10000 });
  const expanded = await toggle.getAttribute("aria-expanded");
  if (expanded !== "true") {
    await toggle.click();
  }
}

test("workbench: archived tasks load after reload", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 0,
    turnsPerSession: 0,
    throttleMs: 0,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1000);

  for (const taskId of seed.taskIds) {
    const resp = await request.post(`/api/tasks/${taskId}/archive`, {});
    expect(resp.ok()).toBeTruthy();
  }

  await page.waitForTimeout(3000);
  await page.reload({ waitUntil: "domcontentloaded" });
  await ensureArchivedExpanded(page);

  await expect(page.locator(".wb-task-row-archived", { hasText: "fixture task 1" })).toBeVisible({
    timeout: 20000,
  });
});
