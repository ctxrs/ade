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

  const archivedEndpoint = `/api/workspaces/${seed.workspaceId}/archived_task_summaries`;
  const seenRequests: string[] = [];
  page.on("request", (req) => {
    const url = req.url();
    if (url.includes(archivedEndpoint)) {
      seenRequests.push(url);
    }
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const firstPageRequest = page.waitForRequest((req) => req.url().includes(archivedEndpoint));
  await page.getByRole("button", { name: "Archived Tasks" }).click();

  await firstPageRequest;
  await expect(page.getByRole("listitem", { name: "fixture task 52" })).toBeVisible({ timeout: 20000 });

  const loadMoreRequest = page.waitForRequest(
    (req) => req.url().includes(archivedEndpoint) && req.url().includes("cursor_"),
  );
  const scroller = page
    .getByRole("list", { name: "Tasks" })
    .locator("xpath=ancestor::*[@data-virtuoso-scroller]")
    .first();
  await scroller.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await loadMoreRequest;

  expect(seenRequests.length).toBeGreaterThan(1);
});
