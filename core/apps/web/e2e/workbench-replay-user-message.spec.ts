import { test, expect } from "./utils/fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: replay restores missed assistant message after stream drop", async ({ page, request }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });
  let blockSnapshot = false;
  let blockActiveSnapshot = false;
  await page.route("**/api/sessions/*/snapshot**", async (route) => {
    if (blockSnapshot) {
      await new Promise((resolve) => setTimeout(resolve, 25000));
    }
    await route.continue();
  });
  await page.route("**/api/workspaces/*/active_snapshot**", async (route) => {
    if (blockActiveSnapshot) {
      await new Promise((resolve) => setTimeout(resolve, 25000));
    }
    await route.continue();
  });

  await page.addInitScript(() => {
    const OriginalWebSocket = window.WebSocket as any;
    (window as any).__contextDropActive = false;
    (window as any).__contextLastStreamWs = null;
    (window as any).__contextStreamOpenCount = 0;
    (window as any).__contextStreamCloseCount = 0;
    (window as any).__contextForceClose = () => {
      try {
        const ws = (window as any).__contextLastStreamWs;
        if (ws) ws.close();
      } catch {
        // ignore
      }
    };

    class DropWindowWebSocket extends OriginalWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        // @ts-expect-error - runtime shim
        super(url, protocols);
        const u = String(url ?? "");
        if (!u.includes("/api/workspaces/") || !u.includes("/active_snapshot/stream")) return;
        (window as any).__contextLastStreamWs = this;
        const originalAddEventListener = this.addEventListener.bind(this);
        this.addEventListener = (type: any, listener: any, options?: any) => {
          if (type === "message") {
            const wrapped = (event: MessageEvent) => {
              if ((window as any).__contextDropActive) return;
              if (typeof listener === "function") return listener.call(this, event);
              if (listener && typeof listener.handleEvent === "function") {
                return listener.handleEvent.call(listener, event);
              }
            };
            return originalAddEventListener(type, wrapped, options);
          }
          return originalAddEventListener(type, listener, options);
        };

        let onMessageHandler: ((event: MessageEvent) => void) | null = null;
        Object.defineProperty(this, "onmessage", {
          get() {
            return onMessageHandler;
          },
          set(handler) {
            onMessageHandler = handler;
          },
        });
        originalAddEventListener("message", (event: MessageEvent) => {
          if ((window as any).__contextDropActive) return;
          if (onMessageHandler) {
            onMessageHandler.call(this, event);
          }
        });

        this.addEventListener("open", () => {
          (window as any).__contextStreamOpenCount += 1;
          if (!(window as any).__contextDropActive) return;
          try {
            this.close();
          } catch {
            // ignore
          }
        });
        this.addEventListener("close", () => {
          (window as any).__contextStreamCloseCount += 1;
        });
      }
    }

    // @ts-expect-error - runtime shim
    window.WebSocket = DropWindowWebSocket;
  });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });
  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const prompt = `missed-assistant-${Date.now()}`;
  const assistantText = `done: ${prompt}`;

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await page.waitForTimeout(400);
  const activeSession = page.locator(".wb-session-slot[aria-hidden=\"false\"]");
  await expect(activeSession.locator("textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });

  await page.waitForFunction(() => (window as any).__contextStreamOpenCount > 0, null, {
    timeout: 10000,
  });
  await page.evaluate(() => {
    (window as any).__contextDropActive = true;
    (window as any).__contextForceClose?.();
  });
  await page.waitForFunction(() => (window as any).__contextStreamCloseCount > 0, null, {
    timeout: 10000,
  });
  await page.waitForTimeout(200);
  blockSnapshot = true;
  blockActiveSnapshot = true;

  const resp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content: prompt, delivery: "immediate" },
  });
  expect(resp.ok()).toBeTruthy();

  await expect
    .poll(async () => {
      const headResp = await request.get(`/api/sessions/${sessionId}/snapshot?limit=50`);
      if (!headResp.ok()) return false;
      const snapshot = (await headResp.json()) as any;
      const msgs = snapshot?.head?.messages ?? [];
      return msgs.some(
        (message: any) =>
          message.role === "assistant" && String(message.content ?? "").includes(prompt),
      );
    })
    .toBe(true);

  const assistantEntry = page.locator(".wb-assistant-entry").filter({ hasText: assistantText });
  await expect(assistantEntry).toHaveCount(0);

  await page.evaluate(() => {
    (window as any).__contextDropActive = false;
    (window as any).__contextForceClose?.();
  });

  await expect(activeSession.locator(".wb-assistant-entry").filter({ hasText: assistantText })).toBeVisible({
    timeout: 20000,
  });

  blockSnapshot = false;
  blockActiveSnapshot = false;
});
