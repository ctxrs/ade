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

  // Stall the first session create request so we can assert the optimistic turn renders
  // immediately (i.e. without waiting on the daemon response).
  let allowFirstCreateSession: (() => void) | null = null;
  const firstCreateSessionGate = new Promise<void>((resolve) => {
    allowFirstCreateSession = resolve;
  });
  let stalledCreateSession = true;
  await page.route("**/api/tasks/*/sessions", async (route) => {
    if (stalledCreateSession && route.request().method() === "POST") {
      stalledCreateSession = false;
      await firstCreateSessionGate;
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

  await page.evaluate((promptText: string) => {
    const w = window as any;
    w.__sendClickAt = performance.now();
    w.__optimisticHeaderSeen = false;
    w.__optimisticHeaderDisappeared = false;
    w.__optimisticHeaderDuplicated = false;
    w.__optimisticHeaderItemId = null;

    const selector = '.wb-session-slot[aria-hidden="false"] .wb-turn-header-content';
    const monitorWindowMs = 1500;
    const startAt = w.__sendClickAt;

    const getMatches = () =>
      Array.from(document.querySelectorAll(selector))
        .filter((node) => (node.textContent ?? "").includes(promptText))
        .map((node) => ({
          node,
          itemId: node.closest("[data-thread-item-id]")?.getAttribute("data-thread-item-id") ?? null,
        }));

    const tick = () => {
      const elapsed = performance.now() - startAt;
      const matches = getMatches();
      const itemIds = matches.map((match) => match.itemId).filter(Boolean) as string[];
      if (!w.__optimisticHeaderSeen && itemIds.length > 0) {
        w.__optimisticHeaderSeen = true;
        w.__optimisticHeaderItemId = itemIds[0];
      }
      if (w.__optimisticHeaderSeen && w.__optimisticHeaderItemId) {
        if (!itemIds.includes(w.__optimisticHeaderItemId)) {
          w.__optimisticHeaderDisappeared = true;
        }
      }
      if (new Set(itemIds).size > 1) w.__optimisticHeaderDuplicated = true;
      if (elapsed < monitorWindowMs) requestAnimationFrame(tick);
    };

    requestAnimationFrame(tick);
  }, prompt);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const header = page
    .locator('.wb-session-slot[aria-hidden="false"] .wb-turn-header-content')
    .filter({ hasText: prompt })
    .first();
  await expect(header).toBeVisible({ timeout: 2000 });
  const headerItemId = await header.evaluate((node) =>
    node.closest("[data-thread-item-id]")?.getAttribute("data-thread-item-id"),
  );
  expect(headerItemId).toBeTruthy();
  const elapsedMs = await page.evaluate(() => performance.now() - (window as any).__sendClickAt);
  expect(elapsedMs).toBeLessThan(300);

  // Release the stalled create-session request now that we verified the optimistic UI.
  allowFirstCreateSession?.();

  await page.evaluate(() => {
    (window as any).__optimisticHeaderAt = performance.now();
  });
  await page.waitForTimeout(400);
  if (headerItemId) {
    await expect(
      page.locator(`[data-thread-item-id="${headerItemId}"] .wb-turn-header-content`),
    ).toBeVisible();
  } else {
    await expect(header).toBeVisible();
  }

  const { queuePanelSeen, shiftAfterHeader, headerDisappeared, headerDuplicated } = await page.evaluate(() => {
    const w = window as any;
    const seenAt = w.__optimisticHeaderAt ?? 0;
    const entries = Array.isArray(w.__layoutShiftEntries) ? w.__layoutShiftEntries : [];
    const shiftAfterHeader = entries
      .filter((entry: any) => Number(entry.startTime) >= seenAt)
      .reduce((sum: number, entry: any) => sum + (Number(entry.value) || 0), 0);
    return {
      queuePanelSeen: Boolean(w.__queuePanelSeen),
      shiftAfterHeader,
      headerDisappeared: Boolean(w.__optimisticHeaderDisappeared),
      headerDuplicated: Boolean(w.__optimisticHeaderDuplicated),
    };
  });

  expect(queuePanelSeen).toBe(false);
  expect(headerDisappeared).toBe(false);
  expect(headerDuplicated).toBe(false);
  // This is a canary for the old "optimistic row removed then re-inserted" behavior.
  // In practice CLS values can be slightly noisy across environments, so keep this
  // threshold lenient enough to avoid flakes while still catching large shifts.
  expect(shiftAfterHeader).toBeLessThan(0.01);
  await expect(page.locator(".queue-panel")).toHaveCount(0);
  await expect(page.locator(".queue-item-content").filter({ hasText: prompt })).toHaveCount(0);
});
