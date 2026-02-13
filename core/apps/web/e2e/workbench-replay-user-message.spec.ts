import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

type E2EWorkspaceStream = {
  getConnectionState?: () => string;
  setDropMessages?: (drop: boolean) => void;
  close?: () => void;
  dispatchMessage?: (payload: unknown) => void;
};

type E2EWindow = Window & {
  __ctxE2E?: {
    workspaceStream?: E2EWorkspaceStream;
    getSessionHeadMessages?: (sessionId: string) => string[];
  };
};

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};

const asArray = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

const readString = (value: unknown): string => (typeof value === "string" ? value : "");

test("workbench: replay restores missed assistant message after stream drop", async ({ page, request }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });
  let blockSnapshot = false;
  let blockHead = false;
  let blockActiveSnapshot = false;
  await page.route("**/api/sessions/*/snapshot**", async (route) => {
    if (blockSnapshot) {
      await new Promise((resolve) => setTimeout(resolve, 25000));
    }
    await route.continue();
  });
  await page.route("**/api/sessions/*/head**", async (route) => {
    if (blockHead) {
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
    window.sessionStorage.setItem("ctxE2E", "1");
  });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });
  const activeSnapshotResp = await request.get(`/api/workspaces/${seed.workspaceId}/active_snapshot`);
  expect(activeSnapshotResp.ok()).toBeTruthy();
  const activeSnapshot = asRecord(await activeSnapshotResp.json());
  const active = asRecord(activeSnapshot.active);
  const activeTaskSummary = asRecord(asArray(active.tasks)[0]);
  const task = asRecord(activeTaskSummary.task);
  const sessions = asArray(activeTaskSummary.sessions).map((entry) => asRecord(entry));
  const primarySessionId = readString(task.primary_session_id);
  const latestSession = sessions.length > 0 ? asRecord(sessions[sessions.length - 1]?.session) : {};
  const latestSessionId = readString(latestSession.id);
  const sessionId = latestSessionId || primarySessionId || seed.sessionIdsByTask[seed.taskIds[0]][0];
  const seedSession =
    sessions
      .find((summary) => readString(asRecord(summary.session).id) === sessionId)
      ?.session ?? asRecord(asRecord(activeTaskSummary.primary_session).session);
  expect(seedSession).toBeTruthy();
  const prompt = `missed-assistant-${Date.now()}`;
  const assistantText = `done: ${prompt}`;

  await page.goto(`/workspaces/${seed.workspaceId}?ctxE2E=1`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();
  await page.waitForTimeout(400);
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });

  await expect
    .poll(async () =>
      page.evaluate(() => typeof (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState === "function"),
    )
    .toBe(true);
  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");
  await page.evaluate(() => {
    (window as E2EWindow).__ctxE2E?.workspaceStream?.setDropMessages?.(true);
  });

  const nowIso = new Date().toISOString();
  const turnId = `turn-${Date.now()}`;
  const runId = `run-${Date.now()}`;
  const assistantMessage = {
    id: `assistant-${Date.now()}-${Math.random().toString(16).slice(2)}`,
    session_id: sessionId,
    task_id: seed.taskIds[0],
    turn_id: turnId,
    turn_sequence: 1,
    role: "assistant",
    content: assistantText,
    attachments: [],
    delivery: "immediate",
    created_at: nowIso,
  };
  const assistantTurn = {
    turn_id: turnId,
    session_id: sessionId,
    run_id: runId,
    user_message_id: null,
    status: "completed",
    start_seq: 1,
    end_seq: 1,
    started_at: nowIso,
    updated_at: nowIso,
    assistant_partial: null,
    thought_partial: null,
    metrics_json: null,
    tool_total: 0,
    tool_pending: 0,
    tool_running: 0,
    tool_completed: 0,
    tool_failed: 0,
  };
  const assistantEvent = {
    id: `event-${Date.now()}`,
    session_id: sessionId,
    task_id: seed.taskIds[0],
    run_id: runId,
    turn_id: turnId,
    event_type: "assistant_message_inserted",
    payload_json: {
      message_id: assistantMessage.id,
      content: assistantText,
      order_seq: 1,
    },
    created_at: nowIso,
  };
  const replayHead = {
    session: seedSession,
    turns: [assistantTurn],
    events: [assistantEvent],
    messages: [assistantMessage],
    last_event_seq: 1,
    state_rev: 0,
    has_more_turns: false,
    has_more_history: false,
    history_cursor: null,
  };
  const replayEvent = {
    type: "session_head_seed",
    workspace_id: seed.workspaceId,
    head: replayHead,
  };

  await page.evaluate((payload) => {
    (window as E2EWindow).__ctxE2E?.workspaceStream?.dispatchMessage?.(payload);
  }, replayEvent);

  blockSnapshot = true;
  blockHead = true;
  blockActiveSnapshot = true;
  await page.evaluate(() => {
    (window as E2EWindow).__ctxE2E?.workspaceStream?.close?.();
  });
  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("disconnected");
  await page.waitForTimeout(200);

  const assistantEntry = page.locator(".wb-assistant-entry").filter({ hasText: assistantText });
  await expect(assistantEntry).toHaveCount(0);

  await page.evaluate(() => {
    (window as E2EWindow).__ctxE2E?.workspaceStream?.setDropMessages?.(false);
    (window as E2EWindow).__ctxE2E?.workspaceStream?.close?.();
  });
  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");
  await page.evaluate((payload) => {
    (window as E2EWindow).__ctxE2E?.workspaceStream?.dispatchMessage?.(payload);
  }, replayEvent);

  await expect
    .poll(
      async () =>
        page.evaluate(
          ({ sessionId: id, text }) =>
            ((window as E2EWindow).__ctxE2E?.getSessionHeadMessages?.(id) ?? []).some((msg: string) =>
              String(msg).includes(text),
            ),
          { sessionId, text: assistantText },
        ),
      { timeout: 20000 },
    )
    .toBe(true);

  blockSnapshot = false;
  blockHead = false;
  blockActiveSnapshot = false;
});
