import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller";

const readId = (v: any): string => (typeof v === "string" ? v : "");

async function addLongMessages(request: any, sessionId: string) {
  const longText = Array.from({ length: 200 }, (_, i) => `fixture line ${i + 1}`).join("\n");
  for (let i = 0; i < 6; i++) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nblock ${i + 1}`, delivery: "immediate" },
    });
  }
}

test("workbench: sticks to bottom unless the user scrolls away", async ({ page, request }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });

  const seed = await seedDummyWorkspace(request, {
    tasks: 0,
    sessionsPerTask: 0,
    turnsPerSession: 0,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });

  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(
    page.locator(".wb-new-composer-stack button[title=\"Harness\"] .wb-switcher-label"),
  ).toHaveText(/fake/i, { timeout: 20000 });

  const initPrompt = `stick-bottom-init-${Date.now()}`;
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(initPrompt);
  await expect(page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]")).toBeEnabled({ timeout: 20000 });
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  let sessionId = "";
  await expect
    .poll(async () => {
      const resp = await request.get(`/api/workspaces/${seed.workspaceId}/active_snapshot`);
      if (!resp.ok()) return "";
      const snapshot = (await resp.json()) as any;
      const taskSummary = snapshot?.active?.tasks?.[0];
      const sessionSummary = taskSummary?.sessions?.[0];
      const resolved =
        readId(sessionSummary?.session?.id) || readId(taskSummary?.task?.primary_session_id);
      sessionId = resolved;
      return resolved;
    }, { timeout: 20000 })
    .not.toBe("");

  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();

  const composer = page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
  await expect(composer).toBeVisible({ timeout: 20000 });

  await addLongMessages(request, sessionId);
  await expect(page.locator(".wb-session")).toContainText("block 6", { timeout: 20000 });

  const scroller = page.locator(scrollSelector).first();
  await expect(scroller).toBeVisible({ timeout: 20000 });
  for (let attempt = 0; attempt < 3; attempt++) {
    const diff = await scroller.evaluate((el) => (el.scrollHeight ?? 0) - (el.clientHeight ?? 0));
    if (diff > 100) break;
    await addLongMessages(request, sessionId);
    await page.waitForTimeout(200);
  }
  await expect
    .poll(async () => scroller.evaluate((el) => (el.scrollHeight ?? 0) - (el.clientHeight ?? 0)), {
      timeout: 10000,
    })
    .toBeGreaterThan(100);

  await expect
    .poll(
      async () => scroller.evaluate((el) => el.scrollHeight - (el.scrollTop + el.clientHeight)),
      { timeout: 10000 },
    )
    .toBeLessThanOrEqual(16);

  const prompt = `stick-bottom-${Date.now()}`;
  await composer.fill(prompt);
  await page.locator(".wb-session-slot[aria-hidden=\"false\"] button[aria-label=\"Send\"]").click();

  await expect(page.locator(".wb-session")).toContainText(prompt, { timeout: 20000 });

  await expect
    .poll(async () => scroller.evaluate((el) => el.scrollHeight - (el.scrollTop + el.clientHeight)), {
      timeout: 10000,
    })
    .toBeLessThanOrEqual(16);

  await scroller.hover();
  await scroller.evaluate((el) => {
    const target = Math.max(0, el.scrollHeight - el.clientHeight - 400);
    el.scrollTop = target;
  });
  await expect
    .poll(async () => scroller.evaluate((el) => el.scrollHeight - (el.scrollTop + el.clientHeight)), {
      timeout: 10000,
    })
    .toBeGreaterThan(200);
  const scrollBefore = await scroller.evaluate((el) => el.scrollTop);
  const jumpToLatest = page.getByRole("button", { name: "Jump to latest" });
  await expect(jumpToLatest).toBeVisible({ timeout: 20000 });

  const incoming = `incoming-${Date.now()}`;
  await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content: incoming, delivery: "immediate" },
  });
  await expect
    .poll(
      async () =>
        page.evaluate(({ id, text }) => {
          const api = (window as any).__ctxE2E;
          return api?.getSessionHeadUserMessages?.(id)?.includes(text) ?? false;
        }, { id: sessionId, text: incoming }),
      { timeout: 20000 },
    )
    .toBe(true);

  const scrollAfter = await scroller.evaluate((el) => el.scrollTop);
  expect(Math.abs(scrollAfter - scrollBefore)).toBeLessThanOrEqual(32);

  await jumpToLatest.click();
  await expect(page.locator(".wb-session")).toContainText(incoming, { timeout: 20000 });
});
