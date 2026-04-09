import { test, expect } from "./fixtures";
import type { APIRequestContext, Locator } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-thread-scroller";
const appendedHistoryTurns = 12;
const oldestFixtureMessagePattern = /\bfixture msg 1\.1\.1\b/;

type ScrollAnchor = { id?: string; offset: number };

async function scrollScrollerUp(
  scroller: Locator,
  deltaPx: number,
): Promise<number> {
  return scroller.evaluate((element, delta) => {
    element.scrollTop = Math.max(0, element.scrollTop - delta);
    element.dispatchEvent(new Event("scroll"));
    return element.scrollTop;
  }, deltaPx);
}

async function addLongMessages(request: APIRequestContext, sessionId: string, count: number) {
  const longText = Array.from({ length: 200 }, (_, i) => `history line ${i + 1}`).join("\n");
  for (let i = 0; i < count; i++) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nhistory scroll ${i + 1}`, delivery: "immediate" },
    });
  }
}

test("workbench: preserves scroll position when prepending history", async ({ page, request }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1200, height: 700 });

  let historyRequestResolve: (() => void) | null = null;
  let historyReleaseResolve: (() => void) | null = null;
  let historyPayloadText: string | null = null;
  const historyRequestSeen = new Promise<void>((resolve) => {
    historyRequestResolve = resolve;
  });
  const historyRelease = new Promise<void>((resolve) => {
    historyReleaseResolve = resolve;
  });
  let historyIntercepted = false;
  await page.route(/\/api\/sessions\/[^/]+\/history/, async (route) => {
    if (historyIntercepted) {
      await route.continue();
      return;
    }
    historyIntercepted = true;
    const response = await route.fetch();
    const body = await response.json();
    historyPayloadText = JSON.stringify(body);
    historyRequestResolve?.();
    await historyRelease;
    await route.fulfill({ response, json: body });
  });
  const historyResponse = page.waitForResponse((response) => response.url().includes("/history"));

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    // Keep the oldest seeded turns outside the default 60-turn authoritative /head window so
    // upward scrolling must fetch an older history page before they become visible.
    turnsPerSession: 55,
    throttleMs: 5,
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId][0];

  await addLongMessages(request, sessionId, appendedHistoryTurns);

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();
  await expect(page.locator("textarea.wb-active-textarea")).toBeVisible({
    timeout: 20000,
  });

  await expect(page.locator(".wb-session")).toContainText(`history scroll ${appendedHistoryTurns}`, { timeout: 20000 });
  await expect(page.locator(".wb-session")).not.toContainText(oldestFixtureMessagePattern, {
    timeout: 1000,
  });

  const scroller = page.locator(scrollSelector).first();
  await expect(scroller).toBeVisible({ timeout: 20000 });

  await expect
    .poll(async () => scroller.evaluate((el) => (el.scrollHeight ?? 0) - (el.clientHeight ?? 0)), {
      timeout: 20000,
    })
    .toBeGreaterThan(100);

  await scroller.evaluate((el) => {
    el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
    el.dispatchEvent(new Event("scroll"));
  });
  await page.waitForTimeout(50);

  for (let i = 0; i < 10; i++) {
    await scrollScrollerUp(scroller, 160);
    await page.waitForTimeout(60);
  }

  let preTop = await scroller.evaluate((el) => el.scrollTop);
  if (preTop <= 0) {
    preTop = await scroller.evaluate((el) => {
      const max = Math.max(0, el.scrollHeight - el.clientHeight);
      const next = Math.max(1, Math.floor(max / 2));
      el.scrollTop = next;
      el.dispatchEvent(new Event("scroll"));
      return el.scrollTop;
    });
  }
  expect(preTop).toBeGreaterThan(0);

  const captureAnchor = async (): Promise<ScrollAnchor | null> => {
    return page.evaluate((selector) => {
      const scrollerEl = document.querySelector(selector);
      if (!scrollerEl) return null;
      const scrollerRect = scrollerEl.getBoundingClientRect();
      const items = Array.from(scrollerEl.querySelectorAll('[role="listitem"]'));
      for (const item of items) {
        const rect = item.getBoundingClientRect();
        if (rect.bottom <= scrollerRect.top + 4) continue;
        const anchorEl = item.querySelector("[data-thread-item-id]") as HTMLElement | null;
        const itemId = anchorEl?.getAttribute("data-thread-item-id");
        if (!itemId) continue;
        return { id: itemId, offset: rect.top - scrollerRect.top };
      }
      return null;
    }, scrollSelector);
  };

  let atTop = false;
  for (let i = 0; i < 60; i++) {
    const top = await scrollScrollerUp(scroller, 200);
    await page.waitForTimeout(60);
    if (top <= 1) {
      atTop = true;
      break;
    }
  }
  if (!atTop) {
    await scroller.evaluate((el) => {
      el.scrollTop = 0;
      el.dispatchEvent(new Event("scroll"));
    });
  }
  await historyRequestSeen;
  await page.waitForTimeout(50);
  const anchorBefore = await captureAnchor();
  expect(anchorBefore).not.toBeNull();
  const anchorId = anchorBefore?.id;
  expect(anchorId).toBeTruthy();
  historyReleaseResolve?.();

  await historyResponse;
  expect(historyPayloadText).toContain("fixture msg 1.1.1");
  await page.waitForTimeout(100);

  const anchorAfter = await page.evaluate(
    ({ selector, itemId }) => {
      const scrollerEl = document.querySelector(selector);
      if (!scrollerEl) return null;
      const scrollerRect = scrollerEl.getBoundingClientRect();
      const anchorEl = scrollerEl.querySelector(`[data-thread-item-id=\"${itemId}\"]`) as HTMLElement | null;
      const item = anchorEl?.closest('[role="listitem"]') as HTMLElement | null;
      if (!item) return null;
      const rect = item.getBoundingClientRect();
      return rect.top - scrollerRect.top;
    },
    { selector: scrollSelector, itemId: anchorId },
  );
  expect(anchorAfter).not.toBeNull();
  expect(Math.abs((anchorAfter as number) - anchorBefore.offset)).toBeLessThanOrEqual(25);

  await scroller.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });
  await expect(page.locator(".wb-session").getByText(oldestFixtureMessagePattern).first()).toBeVisible({
    timeout: 5_000,
  });
});
