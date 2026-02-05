import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller";

async function addLongMessages(request: any, sessionId: string) {
  const longText = Array.from({ length: 240 }, (_, i) => `fixture line ${i + 1}`).join("\n");
  for (let i = 0; i < 8; i++) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nblock ${i + 1}`, delivery: "immediate" },
    });
  }
}

test("workbench: keeps session slots mounted across task switches", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    turnsPerSession: 6,
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
    .poll(async () =>
      scroller.evaluate((el) => (el.scrollHeight ?? 0) - (el.clientHeight ?? 0)),
    )
    .toBeGreaterThan(60);

  const maxTop = await scroller.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
    return el.scrollHeight - el.clientHeight;
  });
  const minAwayFromBottom = 64;

  await page.evaluate(() => {
    (window as any).__slotStats = { min: Number.POSITIVE_INFINITY, max: 0, samples: 0 };
    const sample = () => {
      const count = document.querySelectorAll(".wb-session-slot").length;
      const stats = (window as any).__slotStats;
      stats.min = Math.min(stats.min, count);
      stats.max = Math.max(stats.max, count);
      stats.samples += 1;
    };
    const target = document.querySelector(".wb-session") ?? document.body;
    const observer = new MutationObserver(sample);
    observer.observe(target, { childList: true, subtree: true });
    (window as any).__slotObserver = observer;
    (window as any).__slotTimer = window.setInterval(sample, 50);
    sample();
  });

  await scroller.hover();
  let scrollBefore: { top: number; height: number; client: number; remaining: number } | null = null;
  for (let i = 0; i < 12; i++) {
    await page.mouse.wheel(0, -120);
    await page.waitForTimeout(150);
    const snapshot = await scroller.evaluate((el) => {
      const top = el.scrollTop;
      const height = el.scrollHeight;
      const client = el.clientHeight;
      const remaining = height - (top + client);
      return { top, height, client, remaining };
    });
    if (snapshot.top < maxTop - minAwayFromBottom) {
      scrollBefore = snapshot;
      break;
    }
  }

  expect(scrollBefore).not.toBeNull();
  expect((scrollBefore as any).top).toBeLessThan(maxTop - minAwayFromBottom);
  await page.waitForTimeout(800);

  await taskRowB.click();
  await page.waitForTimeout(600);
  await taskRowA.click();
  await page.waitForTimeout(600);

  const scrollAfter = await page.evaluate((selector) => {
    const el = document.querySelector(selector) as HTMLElement | null;
    if (!el) return null;
    return el.scrollTop;
  }, scrollSelector);

  const slotStats = await page.evaluate(() => {
    const stats = (window as any).__slotStats;
    if (!stats) return null;
    return stats;
  });
  await page.evaluate(() => {
    (window as any).__slotObserver?.disconnect?.();
    if ((window as any).__slotTimer) window.clearInterval((window as any).__slotTimer);
  });

  expect(slotStats).not.toBeNull();
  expect((slotStats as any).min).toBeGreaterThanOrEqual(1);
  expect(scrollAfter).not.toBeNull();
  await expect
    .poll(async () => {
      const top = await page.evaluate((selector) => {
        const el = document.querySelector(selector) as HTMLElement | null;
        if (!el) return null;
        return el.scrollTop;
      }, scrollSelector);
      if (top == null) return Number.POSITIVE_INFINITY;
      return Math.abs(top - scrollBefore.top);
    }, { timeout: 5000 })
    .toBeLessThanOrEqual(32);
});
