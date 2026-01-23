import { test, expect } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: session_gap only refetches the affected session head", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    turnsPerSession: 2,
    throttleMs: 5,
  });

  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];

  await page.addInitScript(() => {
    const OriginalWebSocket = window.WebSocket as any;
    (window as any).__contextActiveStreamWs = null;

    class TrackStreamWebSocket extends OriginalWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        // @ts-expect-error - runtime shim
        super(url, protocols);
        const u = String(url ?? "");
        if (!u.includes("/api/workspaces/") || !u.includes("/active_snapshot/stream")) return;
        (window as any).__contextActiveStreamWs = this;
      }
    }

    // @ts-expect-error - runtime shim
    window.WebSocket = TrackStreamWebSocket;
  });

  const isProviderNoise = (url: string) =>
    url.includes("/api/providers") || url.includes("/api/sessions/web");

  const requests: Array<{ url: string; method: string; ts: number }> = [];
  page.on("request", (req) => {
    const url = req.url();
    if (!url.includes("/api/")) return;
    if (isProviderNoise(url)) return;
    requests.push({ url, method: req.method(), ts: Date.now() });
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await expect(page.locator(".wb-session")).toContainText("fixture msg 1.1.1", { timeout: 20000 });

  await page.waitForFunction(() => (window as any).__contextActiveStreamWs, null, {
    timeout: 10000,
  });
  await page.waitForFunction(
    () => (window as any).__contextActiveStreamWs?.readyState === 1,
    null,
    { timeout: 10000 },
  );

  requests.length = 0;
  const cutoff = Date.now();

  await page.evaluate(
    ({ sessionId, workspaceId }) => {
      const ws = (window as any).__contextActiveStreamWs;
      if (!ws) return;
      const payload = {
        type: "session_gap",
        session_id: sessionId,
        workspace_id: workspaceId,
        after_seq: 999,
      };
      ws.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
    },
    { sessionId, workspaceId: seed.workspaceId },
  );

  await page.waitForResponse((resp) => resp.url().includes(`/api/sessions/${sessionId}/head`));
  await page.waitForTimeout(300);

  const after = requests.filter((r) => r.ts >= cutoff);
  const headRequests = after.filter((r) => r.url.includes(`/api/sessions/${sessionId}/head`));
  expect(headRequests.length).toBeGreaterThan(0);

  const unexpected = after.filter((r) => !r.url.includes(`/api/sessions/${sessionId}/head`));
  expect(unexpected).toEqual([]);
});
