import { test, expect } from "./utils/fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: switching between active tasks is instant (no jank, no loading)", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    turnsPerSession: 3,
    throttleMs: 5,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(2);

  await page.evaluate(() => {
    (window as any).__cls = 0;
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (!entry.hadRecentInput) {
          (window as any).__cls += entry.value;
        }
      }
    }).observe({ type: "layout-shift", buffered: true });
  });

  await rows.nth(0).click();
  await page.waitForTimeout(200);
  await rows.nth(1).click();
  await page.waitForTimeout(400);

  const cls = await page.evaluate(() => (window as any).__cls ?? 0);
  expect(cls).toBeLessThanOrEqual(0.01);
  await expect(page.locator(".wb-session >> text=Loading")).toHaveCount(0);
  await expect(page.locator(".wb-session >> text=Select a track with a session.")).toHaveCount(0);
});
