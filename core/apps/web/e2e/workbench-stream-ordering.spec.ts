import { test, expect } from "./fixtures";
import { seedDummyWorkspace, startStreamingMessages } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller";

const extractIndex = (text: string, prefix: string): number | null => {
  const match = text.match(new RegExp(`${prefix}\\s+(\\d+)`));
  if (!match) return null;
  const value = Number(match[1]);
  return Number.isFinite(value) ? value : null;
};

const readHeaderIndices = async (page: any, prefix: string): Promise<number[]> => {
  const texts = await page.locator(".wb-session .wb-turn-header-content").allTextContents();
  return texts
    .map((text) => extractIndex(text, prefix))
    .filter((value): value is number => Number.isFinite(value));
};

const addLongMessages = async (request: any, sessionId: string, count: number) => {
  const longText = Array.from({ length: 220 }, (_, i) => `stream line ${i + 1}`).join("\n");
  for (let i = 0; i < count; i += 1) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nblock ${i + 1}`, delivery: "immediate" },
    });
  }
};

const captureAnchor = async (page: any): Promise<{ id: string; offset: number } | null> => {
  return page.evaluate((selector) => {
    const scrollerEl = document.querySelector(selector);
    if (!scrollerEl) return null;
    const scrollerRect = scrollerEl.getBoundingClientRect();
    const items = Array.from(scrollerEl.querySelectorAll('[role="listitem"]')) as HTMLElement[];
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

test.describe.serial("workbench: streaming ordering", () => {
  let workspaceId = "";
  let sessionIds: string[] = [];

  test.beforeAll(async ({ request }) => {
    const seed = await seedDummyWorkspace(request, {
      tasks: 2,
      sessionsPerTask: 1,
      turnsPerSession: 8,
      throttleMs: 5,
      messageBytes: 600,
    });
    workspaceId = seed.workspaceId;
    sessionIds = seed.taskIds.map((taskId) => seed.sessionIdsByTask[taskId][0]);
  });

  test("streaming preserves order and avoids duplicates", async ({ page, request }) => {
    const sessionId = sessionIds[0];
    const prefix = `order-msg-${Date.now()}`;

    await page.goto(`/workspaces/${workspaceId}?ctxE2E=1`, { waitUntil: "domcontentloaded" });
    const rows = page.locator(".wb-task-row");
    await expect(rows).toHaveCount(2, { timeout: 20_000 });
    await rows.filter({ hasText: "fixture task 1" }).first().click();
    await expect(page.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({
      timeout: 20_000,
    });

    const stream = startStreamingMessages(request, {
      sessionIds: [sessionId],
      intervalMs: 140,
      durationMs: 1400,
      messagePrefix: prefix,
    });

    await expect
      .poll(async () => (await readHeaderIndices(page, prefix)).length, { timeout: 20_000 })
      .toBeGreaterThanOrEqual(6);

    await stream.stop();
    await page.waitForTimeout(250);

    const indices = await readHeaderIndices(page, prefix);
    expect(indices.length).toBeGreaterThanOrEqual(6);
    expect(new Set(indices).size).toBe(indices.length);
    const sorted = indices.slice().sort((a, b) => a - b);
    expect(indices).toEqual(sorted);
  });

  test("scrollback stays stable while new messages stream", async ({ page, request }) => {
    const sessionId = sessionIds[1];
    const prefix = `scroll-msg-${Date.now()}`;

    await page.goto(`/workspaces/${workspaceId}?ctxE2E=1`, { waitUntil: "domcontentloaded" });
    const rows = page.locator(".wb-task-row");
    await expect(rows).toHaveCount(2, { timeout: 20_000 });
    await rows.filter({ hasText: "fixture task 2" }).first().click();
    await expect(page.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({
      timeout: 20_000,
    });

    await addLongMessages(request, sessionId, 6);

    const scroller = page.locator(scrollSelector).first();
    await expect(scroller).toBeVisible({ timeout: 20_000 });
    await expect
      .poll(async () => scroller.evaluate((el) => el.scrollHeight - el.clientHeight), {
        timeout: 10_000,
      })
      .toBeGreaterThan(100);

    await scroller.evaluate((el) => {
      const maxTop = Math.max(0, el.scrollHeight - el.clientHeight);
      const target = Math.floor(maxTop * 0.4);
      el.scrollTop = target;
      el.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(150);

    const scrollBefore = await scroller.evaluate((el) => el.scrollTop);
    expect(scrollBefore).toBeGreaterThan(0);
    const anchorBefore = await captureAnchor(page);
    expect(anchorBefore).not.toBeNull();

    const stream = startStreamingMessages(request, {
      sessionIds: [sessionId],
      intervalMs: 160,
      durationMs: 1600,
      messagePrefix: prefix,
    });

    await expect
      .poll(async () => (await readHeaderIndices(page, prefix)).length, { timeout: 20_000 })
      .toBeGreaterThanOrEqual(4);

    await stream.stop();
    await page.waitForTimeout(200);

    const anchorAfter = await captureAnchor(page);
    expect(anchorAfter).not.toBeNull();
    if (anchorBefore && anchorAfter && anchorBefore.id === anchorAfter.id) {
      expect(Math.abs(anchorAfter.offset - anchorBefore.offset)).toBeLessThanOrEqual(24);
    }

    await scroller.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
      el.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(200);

    const indices = await readHeaderIndices(page, prefix);
    expect(indices.length).toBeGreaterThanOrEqual(4);
    expect(new Set(indices).size).toBe(indices.length);
    const sorted = indices.slice().sort((a, b) => a - b);
    expect(indices).toEqual(sorted);
  });
});
