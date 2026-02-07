import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: session_gap only refetches the affected session head", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    // Force the UI to open sessions via the SessionReplica (no seeded head content),
    // so a session_gap can trigger a GET /head refetch.
    turnsPerSession: 0,
    throttleMs: 5,
  });

  const sessionIdA = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const sessionIdB = seed.sessionIdsByTask[seed.taskIds[1]][0];

  const requests: Array<{ url: string; method: string; ts: number }> = [];
  page.on("request", (req) => {
    const url = req.url();
    // Only track the head refetch calls we care about.
    if (!url.includes("/api/sessions/") || !url.includes("/head")) return;
    requests.push({ url, method: req.method(), ts: Date.now() });
  });

  await page.goto(`/workspaces/${seed.workspaceId}?ctxE2E=1`, { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("workbench-task-search")).toBeVisible({ timeout: 30000 });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(2, { timeout: 20000 });
  const rowA = rows.filter({ hasText: "fixture task 1" });
  const rowB = rows.filter({ hasText: "fixture task 2" });
  await expect(rowA).toHaveCount(1);
  await expect(rowB).toHaveCount(1);
  await Promise.all([
    page.waitForResponse((resp) => resp.url().includes(`/api/sessions/${sessionIdA}/head`)),
    rowA.click(),
  ]);
  await Promise.all([
    page.waitForResponse((resp) => resp.url().includes(`/api/sessions/${sessionIdB}/head`)),
    rowB.click(),
  ]);

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");
  await expect
    .poll(async () => page.evaluate(() => typeof (window as any).__ctxE2E?.workspaceStream?.dispatchMessage === "function"))
    .toBe(true);

  requests.length = 0;
  const cutoff = Date.now();

  await page.evaluate(
    ({ sessionId, workspaceId }) => {
      const stream = (window as any).__ctxE2E?.workspaceStream;
      if (!stream?.dispatchMessage) return;
      const payload = {
        type: "event",
        event: {
          type: "session_gap",
          session_id: sessionId,
          workspace_id: workspaceId,
          after_seq: 999,
        },
      };
      stream.dispatchMessage(payload);
    },
    // Only the active session is open (refCount > 0), so inject the gap for the active session.
    { sessionId: sessionIdB, workspaceId: seed.workspaceId },
  );

  await expect
    .poll(async () => requests.some((r) => r.url.includes(`/api/sessions/${sessionIdB}/head`)), { timeout: 60_000 })
    .toBe(true);
  await page.waitForTimeout(300);

  const after = requests.filter((r) => r.ts >= cutoff);
  const headRequestsB = after.filter((r) => r.url.includes(`/api/sessions/${sessionIdB}/head`));
  expect(headRequestsB.length).toBeGreaterThan(0);
  const headRequestsA = after.filter((r) => r.url.includes(`/api/sessions/${sessionIdA}/head`));
  expect(headRequestsA.length).toBe(0);
});
