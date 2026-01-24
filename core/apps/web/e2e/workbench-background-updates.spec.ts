import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: background updates land for non-visible active sessions", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    turnsPerSession: 1,
    throttleMs: 5,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(2);

  await rows.nth(0).click();

  const taskId = seed.taskIds[1];
  const sessionId = seed.sessionIdsByTask[taskId][0];
  const msg = `background-${Date.now()}`;
  const resp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content: msg, delivery: "immediate" },
  });
  expect(resp.ok()).toBeTruthy();

  await page.waitForTimeout(300);
  await rows.nth(1).click();

  await expect(page.locator(".wb-session")).toContainText(`done: ${msg}`, { timeout: 20000 });
});
