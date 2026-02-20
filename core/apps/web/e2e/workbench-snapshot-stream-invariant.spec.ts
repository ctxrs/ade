import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import { clearDiagnostics, expectNoUnexpectedDiagnostics, getDiagnostics } from "./utils/diagnostics";
import { expectWsPathOnCanonicalOrigin } from "./utils/wsUrls";
import { selectHarnessBySearch } from "./utils/harnessEndpointAuth";

const readId = (value: unknown): string => (typeof value === "string" ? value : "");
const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
const asArray = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

type E2EWorkspaceStream = {
  getConnectionState?: () => string;
  dispatchMessage?: (payload: unknown) => void;
  close?: () => void;
};

type E2EWindow = Window & {
  __ctxE2E?: {
    workspaceStream?: E2EWorkspaceStream;
    getSessionHeadMessages?: (sessionId: string) => unknown[];
    getSessionLastEventSeq?: (sessionId: string) => number;
  };
};

test("workbench: snapshot+stream invariant keeps active sessions head-free", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    turnsPerSession: 1,
    throttleMs: 5,
  });

  const sessionIdB = seed.sessionIdsByTask[seed.taskIds[1]][0];
  const snapshotResp = await request.get(`/api/workspaces/${seed.workspaceId}/active_snapshot`);
  expect(snapshotResp.ok()).toBeTruthy();
  const snapshot = asRecord(await snapshotResp.json());
  const active = asRecord(snapshot.active);
  const tasks = asArray(active.tasks).map((task) => asRecord(task));
  const taskB =
    tasks.find((task) => {
      const primarySession = asRecord(task.primary_session);
      const primarySessionData = asRecord(primarySession.session);
      const taskData = asRecord(task.task);
      const primarySessionId = readId(primarySessionData.id) || readId(taskData.primary_session_id);
      return primarySessionId === sessionIdB;
    }) ?? null;
  expect(taskB).toBeTruthy();
  const fallbackSummary = asRecord(taskB?.primary_session ?? asArray(taskB?.sessions)[0] ?? null);
  const baseHead = (taskB?.primary_session_head ??
    (Object.keys(fallbackSummary).length > 0
      ? {
          session: fallbackSummary.session,
          turns: [],
          events: [],
          messages: [],
          tool_summaries: [],
          last_event_seq: Number(fallbackSummary.last_event_seq ?? 0),
          state_rev: Number(fallbackSummary.state_rev ?? 0),
          has_more_turns: false,
          has_more_history: false,
          history_cursor: null,
        }
      : null)) as Record<string, unknown> | null;
  expect(asRecord(baseHead).session).toBeTruthy();

  const headRequests: Array<{ url: string; method: string; ts: number }> = [];
  page.on("request", (req) => {
    const url = req.url();
    if (!url.includes("/api/sessions/") || !url.includes("/head")) return;
    headRequests.push({ url, method: req.method(), ts: Date.now() });
  });

  await page.goto(`/workspaces/${seed.workspaceId}?ctxE2E=1`, { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("workbench-task-search")).toBeVisible({ timeout: 30000 });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(2, { timeout: 20000 });
  const rowA = rows.filter({ hasText: "fixture task 1" });
  const rowB = rows.filter({ hasText: "fixture task 2" });
  await expect(rowA).toHaveCount(1);
  await expect(rowB).toHaveCount(1);

  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");
  await expect
    .poll(async () =>
      page.evaluate(() => typeof (window as E2EWindow).__ctxE2E?.workspaceStream?.dispatchMessage === "function"),
    )
    .toBe(true);
  await clearDiagnostics(page);

  await selectHarnessBySearch(page, "fake", /fake/i);

  const cutoff = Date.now();
  const newPrompt = `invariant-new-task-${Date.now()}`;
  const newComposer = page.locator("textarea.wb-composer-textarea").first();
  await expect(newComposer).toBeVisible({ timeout: 20000 });
  await newComposer.fill(newPrompt);
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({
    timeout: 20000,
  });
  await expect(
    page
      .locator(".wb-session-slot[aria-hidden=\"false\"] .wb-turn-header-content")
      .filter({ hasText: newPrompt })
      .first(),
  ).toBeVisible({ timeout: 20000 });

  await rowB.click();
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({
    timeout: 20000,
  });

  await page.evaluate(() => {
    (window as E2EWindow).__ctxE2E?.workspaceStream?.close?.();
  });
  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("disconnected");
  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()), {
      timeout: 30000,
    })
    .toBe("connected");

  const baselineMessages = await page.evaluate(
    (sessionId: string) => (window as E2EWindow).__ctxE2E?.getSessionHeadMessages?.(sessionId) ?? [],
    sessionIdB,
  );
  expect(Array.isArray(baselineMessages)).toBeTruthy();
  expect(baselineMessages.length).toBeGreaterThan(0);
  const baselineLastEventSeq = await page.evaluate(
    (sessionId: string) => Number((window as E2EWindow).__ctxE2E?.getSessionLastEventSeq?.(sessionId) ?? 0),
    sessionIdB,
  );

  await page.evaluate(
    ({ sessionId, workspaceId, afterSeq }) => {
      const stream = (window as E2EWindow).__ctxE2E?.workspaceStream;
      if (!stream?.dispatchMessage) return;
      stream.dispatchMessage({
        type: "event",
        event: {
          type: "session_gap",
          workspace_id: workspaceId,
          session_id: sessionId,
          after_seq: afterSeq,
        },
      });
    },
    { sessionId: sessionIdB, workspaceId: seed.workspaceId, afterSeq: baselineLastEventSeq + 100 },
  );

  await expect
    .poll(
      async () =>
        page.evaluate(
          (sessionId: string) => (window as E2EWindow).__ctxE2E?.getSessionHeadMessages?.(sessionId) ?? [],
          sessionIdB,
        ),
      { timeout: 20000 },
    )
    .toEqual(baselineMessages);

  const after = headRequests.filter((requestItem) => requestItem.ts >= cutoff);
  expect(after).toEqual([]);
  await expect(page.locator(".banner .error").filter({ hasText: /load failed/i })).toHaveCount(0);
  await expectWsPathOnCanonicalOrigin(page, "/api/workspaces/");
  await expectNoUnexpectedDiagnostics(page);
  const streamWarnings = (await getDiagnostics(page)).filter((event) =>
    ["workspace.stream_connect_failed", "workspace.stream_connection_missing"].includes(event.code),
  );
  expect(streamWarnings).toEqual([]);
});
