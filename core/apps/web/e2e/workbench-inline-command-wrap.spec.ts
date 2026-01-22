import { test, expect } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: inline code wraps without horizontal scroll", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await page.waitForTimeout(400);
  await expect(page.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });

  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const longSegment = "--flag=abcdefghijklmnopqrstuvwxyz0123456789";
  const longCommand = `bash -lc "cd core/apps/web && pnpm playwright test ${longSegment.repeat(20)}"`;
  const marker = `wrap-test-${Date.now()}`;
  const content = `${marker} \`${longCommand}\``;

  const resp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content, delivery: "immediate" },
  });
  expect(resp.ok()).toBeTruthy();

  const assistantEntry = page.locator(".wb-assistant-entry").filter({ hasText: `done: ${marker}` });
  await expect(assistantEntry).toBeVisible({ timeout: 20000 });

  await expect(assistantEntry.locator(".codeblock")).toHaveCount(0);
  const resolveWrapMetrics = () =>
    page.evaluate((markerValue) => {
      const entries = Array.from(document.querySelectorAll(".wb-assistant-entry"));
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

  await expect.poll(resolveWrapMetrics, { timeout: 2000 }).not.toBeNull();
  const wrapMetrics = await resolveWrapMetrics();
  expect(wrapMetrics).not.toBeNull();

  if (wrapMetrics) {
    expect(Math.max(wrapMetrics.rectCount, wrapMetrics.estimatedLines)).toBeGreaterThan(1);
  }

  const metrics = await page.evaluate(() => {
    const doc = document.documentElement;
    const body = document.body;
    const sessionView = document.querySelector(".wb-session-view") as HTMLElement | null;
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
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await page.waitForTimeout(400);
  await expect(page.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });

  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const marker = `fenced-test-${Date.now()}`;
  const longLine = "--flag=abcdefghijklmnopqrstuvwxyz0123456789".repeat(40);
  const content = `${marker}\n\n\`\`\`text\n${longLine}\n\`\`\``;

  const resp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content, delivery: "immediate" },
  });
  expect(resp.ok()).toBeTruthy();

  const assistantEntry = page.locator(".wb-assistant-entry").filter({ hasText: `done: ${marker}` });
  await expect(assistantEntry).toBeVisible({ timeout: 20000 });

  const codeblock = assistantEntry.locator(".codeblock");
  await expect(codeblock).toBeVisible();

  const copyButton = assistantEntry.locator(".codeblock-copy");
  await expect(copyButton).toBeVisible();
  await copyButton.click();
  await expect(copyButton).toHaveAttribute("title", "Copied");

  const widths = await page.evaluate(() => {
    const sessionView = document.querySelector(".wb-session-view") as HTMLElement | null;
    const codeblockEl = document.querySelector(".wb-assistant-entry .codeblock") as HTMLElement | null;
    if (!sessionView || !codeblockEl) return null;
    const sessionRect = sessionView.getBoundingClientRect();
    const codeRect = codeblockEl.getBoundingClientRect();
    return {
      sessionWidth: sessionRect.width,
      codeWidth: codeRect.width,
    };
  });

  expect(widths).not.toBeNull();
  if (widths) {
    expect(widths.codeWidth).toBeLessThanOrEqual(widths.sessionWidth + 1);
  }

  const scrollMetrics = await page.evaluate(() => {
    const scroller = document.querySelector(
      ".wb-assistant-entry .codeblock-body > pre, .wb-assistant-entry .codeblock-body > div",
    ) as HTMLElement | null;
    if (!scroller) return null;
    return {
      scrollWidth: scroller.scrollWidth,
      clientWidth: scroller.clientWidth,
    };
  });

  expect(scrollMetrics).not.toBeNull();
  if (scrollMetrics) {
    expect(scrollMetrics.scrollWidth).toBeGreaterThan(scrollMetrics.clientWidth);
  }

  const pageMetrics = await page.evaluate(() => {
    const doc = document.documentElement;
    const body = document.body;
    const sessionView = document.querySelector(".wb-session-view") as HTMLElement | null;
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
