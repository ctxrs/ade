import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller";

test("workbench: composer jank stays stable on third line", async ({ page, request }, testInfo) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 12,
    messageBytes: { min: 180, max: 240 },
    messagePrefix: "composer-jank",
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();

  const composer = page.locator(".wb-session textarea.wb-active-textarea");
  await expect(composer).toBeVisible({ timeout: 20000 });

  const scroller = page.locator(scrollSelector).first();
  await expect(scroller).toBeVisible({ timeout: 20000 });
  await expect
    .poll(async () => scroller.evaluate((el) => el.scrollHeight - el.clientHeight), { timeout: 20000 })
    .toBeGreaterThan(80);

  await scroller.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });

  await composer.fill("line one\nline two\n");
  await scroller.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await expect
    .poll(async () => scroller.evaluate((el) => el.scrollHeight - (el.scrollTop + el.clientHeight)), {
      timeout: 10000,
    })
    .toBeLessThanOrEqual(8);
  await page.waitForTimeout(100);

  await page.evaluate(() => {
    const w = window as any;
    w.__composerJankSamples = [];
    w.__composerJankShiftEntries = [];
    w.__composerJankStart = performance.now();

    const scroller = document.querySelector(
      ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller",
    ) as HTMLElement | null;
    const composer = document.querySelector(
      ".wb-session textarea.wb-active-textarea",
    ) as HTMLTextAreaElement | null;
    if (!scroller || !composer) return;

    const recordSample = () => {
      w.__composerJankSamples.push({
        t: performance.now(),
        top: scroller.scrollTop,
        height: scroller.scrollHeight,
        clientHeight: scroller.clientHeight,
      });
    };

    requestAnimationFrame(recordSample);
    composer.addEventListener("input", recordSample);
    w.__composerJankCleanup = () => {
      composer.removeEventListener("input", recordSample);
    };

    w.__composerJankObserver?.disconnect?.();
    const shiftObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as any[]) {
        w.__composerJankShiftEntries.push({
          startTime: entry.startTime,
          value: entry.value ?? 0,
          hadRecentInput: entry.hadRecentInput ?? false,
        });
      }
    });
    shiftObserver.observe({ type: "layout-shift", buffered: true });
    w.__composerJankObserver = shiftObserver;
  });

  await composer.focus();
  await page.keyboard.type("stable-input", { delay: 30 });
  await page.waitForTimeout(150);

  const metrics = await page.evaluate(() => {
    const w = window as any;
    w.__composerJankCleanup?.();
    w.__composerJankObserver?.disconnect?.();
    const samples = Array.isArray(w.__composerJankSamples) ? w.__composerJankSamples : [];
    const entries = Array.isArray(w.__composerJankShiftEntries) ? w.__composerJankShiftEntries : [];
    const startAt = Number(w.__composerJankStart ?? 0);
    const deltas: number[] = [];
    for (let i = 1; i < samples.length; i += 1) {
      const prev = samples[i - 1]?.top ?? 0;
      const next = samples[i]?.top ?? 0;
      deltas.push(next - prev);
    }
    const clsTotal = entries
      .filter((entry: any) => Number(entry.startTime) >= startAt)
      .reduce((sum: number, entry: any) => sum + (Number(entry.value) || 0), 0);
    const clsNoInput = entries
      .filter((entry: any) => Number(entry.startTime) >= startAt && !entry.hadRecentInput)
      .reduce((sum: number, entry: any) => sum + (Number(entry.value) || 0), 0);
    const maxDelta = deltas.reduce((max: number, value: number) => Math.max(max, Math.abs(value)), 0);
    return {
      samples,
      entries,
      deltas,
      clsTotal,
      clsNoInput,
      maxDelta,
    };
  });

  await testInfo.attach("composer-jank-metrics.json", {
    body: JSON.stringify(metrics, null, 2),
    contentType: "application/json",
  });

  expect(metrics.samples.length).toBeGreaterThan(5);
  expect(metrics.maxDelta).toBeLessThanOrEqual(2);
  expect(metrics.clsTotal).toBeLessThan(0.02);
  expect(metrics.clsNoInput).toBeLessThan(0.001);
});
