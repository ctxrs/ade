import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

type E2EWindow = Window & {
  __ctxE2E?: {
    getSessionHeadMessages?: (sessionId: string) => string[];
    getSessionLastEventSeq?: (sessionId: string) => number | null;
    workspaceStream?: {
      getConnectionState?: () => string | null;
      dispatchMessage?: (payload: unknown) => void;
    };
  };
};

test("workbench: session_gap recovers without active /head refetch", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    // Keep heads lightweight for this scenario.
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
  await rowA.click();
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({
    timeout: 20000,
  });
  await rowB.click();
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({
    timeout: 20000,
  });

  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");
  await expect
    .poll(async () => page.evaluate(() => typeof (window as E2EWindow).__ctxE2E?.getSessionHeadMessages === "function"))
    .toBe(true);
  await expect
    .poll(async () => page.evaluate(() => typeof (window as E2EWindow).__ctxE2E?.getSessionLastEventSeq === "function"))
    .toBe(true);
  await expect
    .poll(async () => page.evaluate(() => typeof (window as E2EWindow).__ctxE2E?.workspaceStream?.dispatchMessage === "function"))
    .toBe(true);

  const baseSeq = await page.evaluate(
    ({ sessionId }) => (window as E2EWindow).__ctxE2E?.getSessionLastEventSeq?.(sessionId) ?? null,
    { sessionId: sessionIdB },
  );
  const seedLastEventSeq = typeof baseSeq === "number" && Number.isFinite(baseSeq) ? baseSeq + 1 : 1;
  const marker = `seed-gap-recovery-${seed.workspaceId}-${sessionIdB}-${seedLastEventSeq}`;

  requests.length = 0;
  const cutoff = Date.now();

  await page.evaluate(
    ({ sessionId, workspaceId }) => {
      const stream = (window as E2EWindow).__ctxE2E?.workspaceStream;
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

  await page.evaluate(
    ({ sessionId, taskId, workspaceId, marker, lastEventSeq }) => {
      const stream = (window as E2EWindow).__ctxE2E?.workspaceStream;
      if (!stream?.dispatchMessage) return;
      const now = new Date().toISOString();
      const payload = {
        type: "event",
        event: {
          type: "session_head_seed",
          workspace_id: workspaceId,
          head: {
            session: {
              id: sessionId,
              task_id: taskId,
            },
            turns: [],
            messages: [
              {
                id: `seed-${lastEventSeq}`,
                role: "user",
                content: marker,
                created_at: now,
                turn_sequence: 1,
              },
            ],
            events: [],
            tool_summaries: [],
            last_event_seq: lastEventSeq,
            state_rev: lastEventSeq,
            window: {
              turn_limit: 0,
              message_limit: 0,
              event_limit: 0,
              byte_limit: 0,
              turn_count: 0,
              message_count: 0,
              event_count: 0,
              bytes: 0,
              truncated: false,
            },
          },
        },
      };
      stream.dispatchMessage(payload);
    },
    {
      sessionId: sessionIdB,
      taskId: seed.taskIds[1],
      workspaceId: seed.workspaceId,
      marker,
      lastEventSeq: seedLastEventSeq,
    },
  );

  await expect
    .poll(
      async () =>
        page.evaluate(
          ({ sessionId, marker }) => {
            const api = (window as E2EWindow).__ctxE2E;
            if (!api?.getSessionHeadMessages || !api.getSessionLastEventSeq) return null;
            const messages = api.getSessionHeadMessages(sessionId) ?? [];
            const seq = api.getSessionLastEventSeq(sessionId);
            return { hasMarker: messages.some((content) => String(content).includes(marker)), seq };
          },
          { sessionId: sessionIdB, marker },
        ),
      { timeout: 15_000 },
    )
    .toEqual({ hasMarker: true, seq: seedLastEventSeq });

  await page.waitForTimeout(600);

  const after = requests.filter((r) => r.ts >= cutoff);
  const headRequestsB = after.filter((r) => r.url.includes(`/api/sessions/${sessionIdB}/head`));
  expect(headRequestsB.length).toBe(0);
  const headRequestsA = after.filter((r) => r.url.includes(`/api/sessions/${sessionIdA}/head`));
  expect(headRequestsA.length).toBe(0);
});
