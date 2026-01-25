import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("ws: recovery keeps all streamed messages across tasks", async ({ page, request }) => {
  await page.addInitScript(() => {
    const OriginalWebSocket = window.WebSocket as any;
    (window as any).__ctxStreamClosedOnce ??= false;
    (window as any).__ctxStreamClosedAt ??= 0;

    class FlakyStreamWebSocket extends OriginalWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        // @ts-expect-error runtime shim
        super(url, protocols);
        const target = String(url ?? "");
        if ((window as any).__ctxStreamClosedOnce) return;
        if (!target.includes("/api/workspaces/") || !target.includes("/active_snapshot/stream")) return;

        this.addEventListener("open", () => {
          setTimeout(() => {
            try {
              (window as any).__ctxStreamClosedOnce = true;
              (window as any).__ctxStreamClosedAt = Date.now();
              this.close();
            } catch {
              // ignore
            }
          }, 50);
        });
      }
    }

    // @ts-expect-error runtime shim
    window.WebSocket = FlakyStreamWebSocket;
  });

  const seed = await seedDummyWorkspace(request, {
    tasks: 4,
    sessionsPerTask: 1,
    turnsPerSession: 2,
    throttleMs: 2,
    includeToolSummaries: false,
  });

  await page.goto(`/workspaces/${seed.workspaceId}?ctxE2E=1`, { waitUntil: "domcontentloaded" });
  const search = page.getByTestId("workbench-task-search");
  await expect(search).toBeVisible({ timeout: 30_000 });

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxStreamClosedAt ?? 0))
    .toBeGreaterThan(0);

  await expect
    .poll(async () =>
      page.evaluate(() => typeof (window as any).__ctxE2E?.getSessionHeadMessages === "function"),
    )
    .toBe(true);

  const taskIds = seed.taskIds.slice(0, 3);
  const sessionIds = taskIds.map((taskId) => seed.sessionIdsByTask[taskId][0]);
  const perSession = 4;

  for (let sessionIndex = 0; sessionIndex < sessionIds.length; sessionIndex += 1) {
    const sessionId = sessionIds[sessionIndex];
    for (let messageIndex = 0; messageIndex < perSession; messageIndex += 1) {
      const marker = `gap-${sessionIndex + 1}-${messageIndex + 1}`;
      const response = await request.post(`/api/sessions/${sessionId}/messages`, {
        data: {
          content: marker,
          delivery: "immediate",
        },
      });
      expect(response.ok()).toBe(true);
    }
  }

  const expectedBySession = sessionIds.map((_, sessionIndex) =>
    Array.from({ length: perSession }, (_, messageIndex) => `gap-${sessionIndex + 1}-${messageIndex + 1}`),
  );

  await expect
    .poll(async () =>
      page.evaluate(
        ({ sessionIds: ids, expected }) => {
          const api = (window as any).__ctxE2E;
          if (!api || typeof api.getSessionHeadMessages !== "function") {
            return expected.flat();
          }
          const missing: string[] = [];
          for (let i = 0; i < ids.length; i += 1) {
            const messages: string[] = api.getSessionHeadMessages(ids[i]) ?? [];
            for (const marker of expected[i]) {
              if (!messages.some((content) => typeof content === "string" && content.includes(marker))) {
                missing.push(marker);
              }
            }
          }
          return missing;
        },
        { sessionIds, expected: expectedBySession },
      ),
    )
    .toEqual([]);
});
