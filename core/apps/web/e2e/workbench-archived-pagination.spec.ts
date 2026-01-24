import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: archived pagination uses workspace task listing", async ({ page, request }) => {
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
    if (url.includes(`/api/workspaces/${seed.workspaceId}/tasks`)) {
      seenRequests.push(url);
    }
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  await page.getByRole("button", { name: "Archived" }).click();

  await page.waitForRequest((req) => req.url().includes(`/api/workspaces/${seed.workspaceId}/tasks`));
  await expect(page.locator(".wb-task-row-archived").first()).toBeVisible({ timeout: 20000 });

  const scroller = page.locator(".wb-task-scroll");
  await scroller.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });

  expect(seenRequests.length).toBeGreaterThan(0);
});
