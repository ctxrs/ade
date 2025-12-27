import { test, expect } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector =
  ".wb-session [data-virtuoso-scroller], .wb-session [data-viewport-type=\"element\"], .wb-session .thread-stack";

async function addLongMessages(request: any, sessionId: string) {
  const longText = Array.from({ length: 120 }, (_, i) => `fixture line ${i + 1}`).join("\n");
  for (let i = 0; i < 6; i++) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nblock ${i + 1}`, delivery: "immediate" },
    });
  }
}

test("workbench: restores scroll position across session switches", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    turnsPerSession: 12,
    throttleMs: 5,
  });

  const [taskA, taskB] = seed.taskIds;
  const sessionA = seed.sessionIdsByTask[taskA]?.[0];
  const sessionB = seed.sessionIdsByTask[taskB]?.[0];
  expect(sessionA).toBeTruthy();
  expect(sessionB).toBeTruthy();

  await addLongMessages(request, sessionA);
  await addLongMessages(request, sessionB);

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(2);

  const taskRowA = rows.filter({ hasText: "fixture task 1" }).first();
  const taskRowB = rows.filter({ hasText: "fixture task 2" }).first();

  await taskRowA.click();
  await page.waitForSelector(scrollSelector);

  const scrollBefore = await page.evaluate((selector) => {
    const el = document.querySelector(selector) as HTMLElement | null;
    if (!el) return null;
    el.scrollTop = Math.floor(el.scrollHeight * 0.4);
    return { top: el.scrollTop, height: el.scrollHeight, client: el.clientHeight };
  }, scrollSelector);
  expect(scrollBefore).not.toBeNull();
  await page.waitForTimeout(300);

  await page.evaluate(() => {
    (window as any).__cls = 0;
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (!entry.hadRecentInput) (window as any).__cls += entry.value;
      }
    }).observe({ type: "layout-shift", buffered: true });
  });

  await taskRowB.click();
  await page.waitForTimeout(600);
  await taskRowA.click();
  await page.waitForTimeout(600);

  const scrollAfter = await page.evaluate((selector) => {
    const el = document.querySelector(selector) as HTMLElement | null;
    if (!el) return null;
    return el.scrollTop;
  }, scrollSelector);

  const cls = await page.evaluate(() => (window as any).__cls ?? 0);
  expect(cls).toBeLessThanOrEqual(0.02);
  expect(scrollAfter).not.toBeNull();
  expect(Math.abs((scrollAfter as number) - (scrollBefore as any).top)).toBeLessThanOrEqual(32);
});
