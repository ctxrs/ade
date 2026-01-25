import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: optimistic new task message skips queued UI", async ({ page }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;
  await createWorkspaceAndOpenWorkbench({ page, request: page.request, repo, workspaceName });

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();

  let delayFirstMessage = true;
  await page.route("**/api/sessions/*/messages", async (route) => {
    if (delayFirstMessage && route.request().method() === "POST") {
      delayFirstMessage = false;
      await new Promise((resolve) => setTimeout(resolve, 900));
    }
    await route.continue();
  });

  const prompt = `optimistic-${Date.now()}`;
  const composer = page.locator(".wb-new-composer-stack textarea.wb-composer-textarea");
  await expect(composer).toBeVisible({ timeout: 20000 });
  await composer.fill(prompt);

  await page.evaluate(() => {
    const w = window as any;
    w.__queuePanelSeen = false;
    w.__queuePanelObserver?.disconnect?.();
    const queueObserver = new MutationObserver(() => {
      if (document.querySelector(".queue-panel")) {
        w.__queuePanelSeen = true;
      }
    });
    queueObserver.observe(document.body, { childList: true, subtree: true, attributes: true });
    w.__queuePanelObserver = queueObserver;

    w.__layoutShiftEntries = [];
    w.__layoutShiftObserver?.disconnect?.();
    const shiftObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as any[]) {
        w.__layoutShiftEntries.push({
          startTime: entry.startTime,
          value: entry.value ?? 0,
          hadRecentInput: entry.hadRecentInput ?? false,
        });
      }
    });
    shiftObserver.observe({ type: "layout-shift", buffered: false });
    w.__layoutShiftObserver = shiftObserver;
  });

  await page.evaluate(() => {
    (window as any).__sendClickAt = performance.now();
  });
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const header = page.locator(".wb-turn-header-content").filter({ hasText: prompt });
  await expect(header).toBeVisible({ timeout: 2000 });
  const elapsedMs = await page.evaluate(() => performance.now() - (window as any).__sendClickAt);
  expect(elapsedMs).toBeLessThan(800);

  await page.evaluate(() => {
    (window as any).__optimisticHeaderAt = performance.now();
  });
  await page.waitForTimeout(400);

  const { queuePanelSeen, shiftAfterHeader } = await page.evaluate(() => {
    const w = window as any;
    const seenAt = w.__optimisticHeaderAt ?? 0;
    const entries = Array.isArray(w.__layoutShiftEntries) ? w.__layoutShiftEntries : [];
    const shiftAfterHeader = entries
      .filter((entry: any) => Number(entry.startTime) >= seenAt)
      .reduce((sum: number, entry: any) => sum + (Number(entry.value) || 0), 0);
    return {
      queuePanelSeen: Boolean(w.__queuePanelSeen),
      shiftAfterHeader,
    };
  });

  expect(queuePanelSeen).toBe(false);
  expect(shiftAfterHeader).toBeLessThan(0.001);
  await expect(page.locator(".queue-panel")).toHaveCount(0);
  await expect(page.locator(".queue-item-content").filter({ hasText: prompt })).toHaveCount(0);
});
