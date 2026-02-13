import { test, expect } from "./fixtures";
import type { APIRequestContext } from "@playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector =
  ".wb-session-slot[aria-hidden=\"false\"] [data-virtuoso-scroller], .wb-session-slot[aria-hidden=\"false\"] [data-viewport-type=\"element\"], .wb-session-slot[aria-hidden=\"false\"] .thread-stack";

type ScrollRestoreWindow = Window & { __cls?: number };

async function addLongMessages(request: APIRequestContext, sessionId: string) {
  const longText = Array.from({ length: 120 }, (_, i) => `fixture line ${i + 1}`).join("\n");
  for (let i = 0; i < 6; i++) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nblock ${i + 1}`, delivery: "immediate" },
    });
    await new Promise((r) => setTimeout(r, 25));
  }
  await new Promise((r) => setTimeout(r, 400));
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

  const scroller = page.locator(scrollSelector).first();
  await expect
    .poll(async () => scroller.evaluate((el) => el.scrollHeight), { timeout: 10_000 })
    .toBeGreaterThan(0);

  await scroller.hover();
  await page.mouse.wheel(0, 600);
  await page.waitForTimeout(300);

  const scrollBefore = await scroller.evaluate((el): { top: number; height: number; client: number } => ({
    top: el.scrollTop,
    height: el.scrollHeight,
    client: el.clientHeight,
  }));
  expect(scrollBefore).not.toBeNull();
  await page.waitForTimeout(300);

  await page.evaluate(() => {
    const win = window as ScrollRestoreWindow;
    win.__cls = 0;
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        const layoutShiftEntry = entry as PerformanceEntry & { hadRecentInput?: boolean; value?: number };
        if (!layoutShiftEntry.hadRecentInput) {
          win.__cls = (win.__cls ?? 0) + (layoutShiftEntry.value ?? 0);
        }
      }
    }).observe({ type: "layout-shift", buffered: true });
  });

  await taskRowB.click();
  await page.waitForTimeout(600);
  await page.evaluate(() => {
    (window as ScrollRestoreWindow).__cls = 0;
  });
  await taskRowA.click();
  await page.waitForTimeout(600);

  const scrollAfter = await page.evaluate((selector) => {
    const el = document.querySelector(selector) as HTMLElement | null;
    if (!el) return null;
    return el.scrollTop;
  }, scrollSelector);

  const cls = await page.evaluate(() => (window as ScrollRestoreWindow).__cls ?? 0);
  expect(cls).toBeLessThanOrEqual(0.1);
  expect(scrollAfter).not.toBeNull();
  expect(Math.abs((scrollAfter as number) - scrollBefore.top)).toBeLessThanOrEqual(32);
});
