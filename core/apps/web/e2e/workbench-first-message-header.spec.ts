import { test, expect } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: first user message renders from stream when head is stale", async ({ page, request }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });

  await page.addInitScript(() => {
    const OriginalWebSocket = window.WebSocket as any;
    (window as any).__contextStreamOpenCount = 0;

    class TrackWebSocket extends OriginalWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        // @ts-expect-error runtime shim
        super(url, protocols);
        const u = String(url ?? "");
        if (!u.includes("/api/workspaces/") || !u.includes("/stream")) return;
        this.addEventListener("open", () => {
          (window as any).__contextStreamOpenCount += 1;
        });
      }
    }

    // @ts-expect-error runtime shim
    window.WebSocket = TrackWebSocket;
  });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });
  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const prompt = `first-message-${Date.now()}`;
  let forceStaleHead = true;

  await page.route("**/api/sessions/*/head**", async (route) => {
    const url = route.request().url();
    if (!url.includes(sessionId)) {
      await route.continue();
      return;
    }

    const response = await route.fetch();
    if (!forceStaleHead) {
      await route.fulfill({ response });
      return;
    }

    const head = (await response.json()) as any;
    const staleHead = {
      ...head,
      turns: [],
      messages: [],
      events: [],
      tool_summaries: [],
      has_more_turns: false,
      last_event_seq: 0,
    };
    await route.fulfill({ response, body: JSON.stringify(staleHead) });
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();

  const composer = page.locator(".wb-session textarea.wb-active-textarea");
  await expect(composer).toBeVisible({ timeout: 20000 });

  await page.waitForFunction(() => (window as any).__contextStreamOpenCount > 0, null, {
    timeout: 10000,
  });

  await composer.fill(prompt);
  await page.locator(".wb-session button[aria-label=\"Send\"]").click();

  const header = page.locator(".wb-turn-header-content").filter({ hasText: prompt });
  await expect(header).toBeVisible({ timeout: 20000 });

  forceStaleHead = false;
});
