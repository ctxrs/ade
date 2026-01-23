import { test, expect } from "playwright/test";
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
  const sessionView = page.locator(".wb-session");
  const firstMarker = "fixture msg 1.1.1";
  const secondMarker = "fixture msg 2.1.1";
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
  await expect(sessionView).toContainText(firstMarker, { timeout: 20000 });
  await rows.nth(1).click();
  await expect(sessionView).toContainText(secondMarker, { timeout: 20000 });

  await page.evaluate(() => {
    (window as any).__cls = 0;
  });

  const measureSwitch = async (rowIndex: number, marker: string) => {
    const start = await page.evaluate(() => performance.now());
    await rows.nth(rowIndex).click();
    await expect(sessionView).toContainText(marker, { timeout: 20000 });
    const end = await page.evaluate(() => performance.now());
    return end - start;
  };

  const latencyA = await measureSwitch(0, firstMarker);
  const latencyB = await measureSwitch(1, secondMarker);
  const maxLatencyMs = 120;
  expect(Math.max(latencyA, latencyB)).toBeLessThanOrEqual(maxLatencyMs);

  const cls = await page.evaluate(() => (window as any).__cls ?? 0);
  expect(cls).toBeLessThanOrEqual(0.01);
  await expect(page.locator(".wb-session >> text=Loading")).toHaveCount(0);
  await expect(page.locator(".wb-session >> text=Select a track with a session.")).toHaveCount(0);
});
