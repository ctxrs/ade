import { test, expect } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: archived pagination uses archived-only catchup", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 52,
    sessionsPerTask: 0,
    turnsPerSession: 0,
    throttleMs: 0,
  });

  for (const taskId of seed.taskIds) {
    const resp = await request.post(`/api/tasks/${taskId}/archive`, {});
    expect(resp.ok()).toBeTruthy();
  }

  const seenRequests: string[] = [];
  page.on("request", (req) => {
    const url = req.url();
    if (url.includes(`/api/workspaces/${seed.workspaceId}/catchup`)) {
      seenRequests.push(url);
    }
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  await page.getByRole("button", { name: "Archived" }).click();

  const loadMore = page.getByRole("button", { name: "Load more" });
  await expect(loadMore).toBeVisible({ timeout: 20000 });
  const waitForArchived = page.waitForRequest((req) => req.url().includes("archived_only=1"));
  await loadMore.click();
  await waitForArchived;

  const archivedOnlyRequests = seenRequests.filter((url) => url.includes("archived_only=1"));
  expect(archivedOnlyRequests.length).toBeGreaterThan(0);
});
