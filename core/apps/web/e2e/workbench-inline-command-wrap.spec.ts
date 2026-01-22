import { test, expect } from "./utils/fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: inline code wraps without horizontal scroll", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();
  await page.waitForTimeout(400);
  const activeSession = page.locator(".wb-session-slot[aria-hidden=\"false\"]");
  await expect(activeSession.locator("textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });

  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const longSegment = "--flag=abcdefghijklmnopqrstuvwxyz0123456789";
  const longCommand = `bash -lc "cd core/apps/web && pnpm playwright test ${longSegment.repeat(20)}"`;
  const marker = `wrap-test-${Date.now()}`;
  const content = `${marker} \`${longCommand}\``;

  const resp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content, delivery: "immediate" },
  });
  expect(resp.ok()).toBeTruthy();

  const assistantEntry = activeSession
    .locator(".wb-assistant-entry")
    .filter({ hasText: `done: ${marker}` })
    .first();
  await expect(assistantEntry).toBeVisible({ timeout: 20000 });

  await expect(assistantEntry.locator(".codeblock")).toHaveCount(0);
  const resolveWrapMetrics = () =>
    page.evaluate((markerValue) => {
      const activeSlot = document.querySelector(".wb-session-slot[aria-hidden=\"false\"]");
      const entries = Array.from(activeSlot?.querySelectorAll(".wb-assistant-entry") ?? []);
      const entry = entries.find((el) => el.textContent?.includes(`done: ${markerValue}`));
      const code = entry?.querySelector("code");
      if (!code) return null;
      const style = window.getComputedStyle(code);
      const lineHeight = Number.parseFloat(style.lineHeight);
      const rectCount = code.getClientRects().length;
      const boxHeight = code.getBoundingClientRect().height;
      const estimatedLines = Number.isFinite(lineHeight) && lineHeight > 0 ? Math.round(boxHeight / lineHeight) : 0;
      return { rectCount, estimatedLines };
    }, marker);

  await expect
    .poll(async () => {
      const metrics = await resolveWrapMetrics();
      if (!metrics) return 0;
      return Math.max(metrics.rectCount, metrics.estimatedLines);
    }, { timeout: 10000 })
    .toBeGreaterThan(1);

  const metrics = await page.evaluate(() => {
    const doc = document.documentElement;
    const body = document.body;
    const sessionSlot = document.querySelector(".wb-session-slot[aria-hidden=\"false\"]");
    const sessionView = sessionSlot?.querySelector(".wb-session-view") as HTMLElement | null;
    return {
      docScrollWidth: doc.scrollWidth,
      docClientWidth: doc.clientWidth,
      bodyScrollWidth: body.scrollWidth,
      bodyClientWidth: body.clientWidth,
      sessionScrollWidth: sessionView?.scrollWidth ?? 0,
      sessionClientWidth: sessionView?.clientWidth ?? 0,
    };
  });

  expect(metrics.docScrollWidth).toBeLessThanOrEqual(metrics.docClientWidth + 1);
  expect(metrics.bodyScrollWidth).toBeLessThanOrEqual(metrics.bodyClientWidth + 1);
  expect(metrics.sessionScrollWidth).toBeLessThanOrEqual(metrics.sessionClientWidth + 1);
});

test("workbench: fenced code blocks stay within thread width", async ({ page, request }) => {
  await page.context().grantPermissions(["clipboard-write"]);
  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();
  await page.waitForTimeout(400);
  const activeSession = page.locator(".wb-session-slot[aria-hidden=\"false\"]");
  await expect(activeSession.locator("textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });

  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const marker = `fenced-test-${Date.now()}`;
  const longLine = "--flag=abcdefghijklmnopqrstuvwxyz0123456789".repeat(40);
  const content = `${marker}\n\n\`\`\`text\n${longLine}\n\`\`\``;

  const resp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content, delivery: "immediate" },
  });
  expect(resp.ok()).toBeTruthy();

  const assistantEntry = activeSession
    .locator(".wb-assistant-entry")
    .filter({ hasText: `done: ${marker}` })
    .first();
  await expect(assistantEntry).toBeVisible({ timeout: 20000 });

  const codeblock = assistantEntry.locator(".codeblock");
  await expect(codeblock).toBeVisible();

  const copyButton = assistantEntry.locator(".codeblock-copy");
  await expect(copyButton).toBeVisible();
  await copyButton.click();
  await expect(copyButton).toHaveAttribute("title", "Copied");

  const sessionView = activeSession.locator(".wb-session-view");
  await expect(sessionView).toBeVisible();
  const sessionBox = await sessionView.boundingBox();
  const codeBox = await codeblock.boundingBox();
  expect(sessionBox).not.toBeNull();
  expect(codeBox).not.toBeNull();
  if (sessionBox && codeBox) {
    expect(codeBox.width).toBeLessThanOrEqual(sessionBox.width + 1);
  }

  const scroller = assistantEntry.locator(".codeblock-body > pre, .codeblock-body > div").first();
  await expect(scroller).toBeVisible();
  const scrollMetrics = await scroller.evaluate((el) => ({
    scrollWidth: el.scrollWidth,
    clientWidth: el.clientWidth,
  }));
  expect(scrollMetrics.scrollWidth).toBeGreaterThan(scrollMetrics.clientWidth);

  const pageMetrics = await page.evaluate(() => {
    const doc = document.documentElement;
    const body = document.body;
    const sessionSlot = document.querySelector(".wb-session-slot[aria-hidden=\"false\"]");
    const sessionView = sessionSlot?.querySelector(".wb-session-view") as HTMLElement | null;
    return {
      docScrollWidth: doc.scrollWidth,
      docClientWidth: doc.clientWidth,
      bodyScrollWidth: body.scrollWidth,
      bodyClientWidth: body.clientWidth,
      sessionScrollWidth: sessionView?.scrollWidth ?? 0,
      sessionClientWidth: sessionView?.clientWidth ?? 0,
    };
  });

  expect(pageMetrics.docScrollWidth).toBeLessThanOrEqual(pageMetrics.docClientWidth + 1);
  expect(pageMetrics.bodyScrollWidth).toBeLessThanOrEqual(pageMetrics.bodyClientWidth + 1);
  expect(pageMetrics.sessionScrollWidth).toBeLessThanOrEqual(pageMetrics.sessionClientWidth + 1);
});
