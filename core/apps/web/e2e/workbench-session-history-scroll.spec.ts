import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller";

async function addLongMessages(request: any, sessionId: string, count: number) {
  const longText = Array.from({ length: 200 }, (_, i) => `history line ${i + 1}`).join("\n");
  for (let i = 0; i < count; i++) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nhistory scroll ${i + 1}`, delivery: "immediate" },
    });
  }
}

test("workbench: infinite scroll loads older session history", async ({ page, request }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1200, height: 700 });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 6,
    throttleMs: 5,
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId][0];

  await addLongMessages(request, sessionId, 6);

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();
  await expect(page.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({
    timeout: 20000,
  });

  await expect(page.locator(".wb-session")).toContainText("history scroll 6", { timeout: 20000 });
  await expect(page.locator(".wb-session")).not.toContainText("fixture msg 1.1.1", {
    timeout: 1000,
  });

  const scroller = page.locator(scrollSelector).first();
  await expect(scroller).toBeVisible({ timeout: 20000 });
  await expect
    .poll(async () => scroller.evaluate((el) => (el.scrollHeight ?? 0) - (el.clientHeight ?? 0)), {
      timeout: 10000,
    })
    .toBeGreaterThan(100);

  const historyResponse = page.waitForResponse((resp) => {
    return resp.url().includes(`/api/sessions/${sessionId}/history`) && resp.ok();
  });

  await scroller.hover();
  for (let i = 0; i < 16; i++) {
    await page.mouse.wheel(0, -120);
    await page.waitForTimeout(80);
  }

  await historyResponse;
  await scroller.evaluate((el) => {
    el.scrollTop = 0;
  });

  await expect(page.locator(".wb-session")).toContainText("fixture msg 1.1.1", {
    timeout: 20000,
  });
});
