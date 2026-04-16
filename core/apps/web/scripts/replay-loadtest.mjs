import { chromium } from "playwright";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(__dirname, "..");

const defaultFixture = path.resolve(appRoot, "e2e/fixtures/workbench-replay-fixture.json");
const defaultOut = path.join(os.tmpdir(), "ctx-web-loadtest-telemetry.json");
const defaultBaseUrl = "http://127.0.0.1:5173";

const args = process.argv.slice(2);
let fixturePath = defaultFixture;
let outPath = defaultOut;
let baseUrl = process.env.CTX_LOADTEST_BASE_URL || defaultBaseUrl;
let check = false;
let maxSessionSwitchP95 = null;
let maxSessionSwitchP99 = null;
let maxLongTaskMs = null;
let maxLongTaskCount = null;
let synthesizeDeltas = null;
let synthesizeIntervalMs = 1;
let synthesizeStartDelayMs = 200;
let synthesizeMessageBytes = null;
let synthesizeSessionIds = null;
let synthesizeSummaryDeltas = null;
let synthesizeSummaryIntervalMs = 1;
let synthesizeSummaryStartDelayMs = 200;
let synthesizeSummaryMessageBytes = null;
let synthesizeSummarySessionIds = null;
let synthesizeTaskDeltas = null;
let synthesizeTaskIntervalMs = 1;
let synthesizeTaskStartDelayMs = 200;
let synthesizeTaskIds = null;
let synthesizeForegroundTerminalDelayMs = null;
let synthesizeForegroundTerminalSessionId = null;
let resetSessionHeadToRunningId = null;
let waitTimeoutMs = null;
let skipWaitForSynth = false;

for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === "--fixture") {
    fixturePath = args[i + 1] || fixturePath;
    i += 1;
    continue;
  }
  if (arg === "--out") {
    outPath = args[i + 1] || outPath;
    i += 1;
    continue;
  }
  if (arg === "--base-url") {
    baseUrl = args[i + 1] || baseUrl;
    i += 1;
    continue;
  }
  if (arg === "--check") {
    check = true;
    continue;
  }
  if (arg === "--max-session-switch-p95") {
    maxSessionSwitchP95 = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--max-session-switch-p99") {
    maxSessionSwitchP99 = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--max-long-task-ms") {
    maxLongTaskMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--max-long-task-count") {
    maxLongTaskCount = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-deltas") {
    synthesizeDeltas = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-interval-ms") {
    synthesizeIntervalMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-start-delay-ms") {
    synthesizeStartDelayMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-message-bytes") {
    synthesizeMessageBytes = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-session-ids") {
    synthesizeSessionIds = (args[i + 1] || "")
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-summary-deltas") {
    synthesizeSummaryDeltas = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-summary-interval-ms") {
    synthesizeSummaryIntervalMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-summary-start-delay-ms") {
    synthesizeSummaryStartDelayMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-summary-message-bytes") {
    synthesizeSummaryMessageBytes = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-summary-session-ids") {
    synthesizeSummarySessionIds = (args[i + 1] || "")
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-foreground-terminal-delay-ms") {
    synthesizeForegroundTerminalDelayMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-task-deltas") {
    synthesizeTaskDeltas = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-task-interval-ms") {
    synthesizeTaskIntervalMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-task-start-delay-ms") {
    synthesizeTaskStartDelayMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-task-ids") {
    synthesizeTaskIds = (args[i + 1] || "")
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    i += 1;
    continue;
  }
  if (arg === "--synthesize-foreground-terminal-session-id") {
    synthesizeForegroundTerminalSessionId = String(args[i + 1] || "").trim() || null;
    i += 1;
    continue;
  }
  if (arg === "--reset-session-head-to-running") {
    resetSessionHeadToRunningId = String(args[i + 1] || "").trim() || null;
    i += 1;
    continue;
  }
  if (arg === "--wait-timeout-ms") {
    waitTimeoutMs = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--no-wait-for-synth") {
    skipWaitForSynth = true;
    continue;
  }
  if (arg === "--help" || arg === "-h") {
    console.log(
      "Usage: node ./scripts/replay-loadtest.mjs [--fixture path] [--out path] [--base-url url] " +
        "[--check] [--max-session-switch-p95 ms] [--max-session-switch-p99 ms] " +
        "[--max-long-task-ms ms] [--max-long-task-count n] " +
        "[--synthesize-deltas n] [--synthesize-interval-ms ms] [--synthesize-start-delay-ms ms] " +
        "[--synthesize-message-bytes n] [--synthesize-session-ids id1,id2] " +
        "[--synthesize-summary-deltas n] [--synthesize-summary-interval-ms ms] " +
        "[--synthesize-summary-start-delay-ms ms] [--synthesize-summary-message-bytes n] " +
        "[--synthesize-summary-session-ids id1,id2] " +
        "[--synthesize-task-deltas n] [--synthesize-task-interval-ms ms] " +
        "[--synthesize-task-start-delay-ms ms] [--synthesize-task-ids id1,id2] " +
        "[--synthesize-foreground-terminal-delay-ms ms] [--synthesize-foreground-terminal-session-id id] " +
        "[--reset-session-head-to-running id] " +
        "[--wait-timeout-ms ms] [--no-wait-for-synth]\n",
    );
    process.exit(0);
  }
}

const percentile = (values, pct) => {
  if (!values.length) return null;
  const sorted = values.slice().sort((a, b) => a - b);
  const rank = Math.ceil(pct * sorted.length) - 1;
  const idx = Math.min(sorted.length - 1, Math.max(0, rank));
  return sorted[idx];
};

const summarizeValues = (values) => ({
  count: values.length,
  p50: percentile(values, 0.5),
  p95: percentile(values, 0.95),
  p99: percentile(values, 0.99),
  max: values.length ? Math.max(...values) : null,
});

const fixture = JSON.parse(await readFile(fixturePath, "utf8"));
const workspaceId = fixture?.workspace?.id || fixture?.active_snapshot?.workspace_id;
if (!workspaceId) {
  throw new Error("Fixture missing workspace id.");
}

let streamEvents = Array.isArray(fixture.stream) ? fixture.stream : [];
const snapshotBySession = fixture.session_snapshots || {};
const activeHeads = Array.isArray(fixture.active_heads?.heads)
  ? fixture.active_heads.heads
  : Object.values(snapshotBySession)
      .map((snapshot) => snapshot?.head)
      .filter(Boolean);
const activeTasks = Array.isArray(fixture.active_snapshot?.active?.tasks)
  ? fixture.active_snapshot.active.tasks
  : [];
const activeHeadsBatch = {
  workspace_id: workspaceId,
  snapshot_rev: fixture.active_snapshot?.snapshot_rev ?? 0,
  heads: activeHeads,
};
const clientTelemetryEvents = [];
const normalizedProviders = (fixture.providers ?? []).map((provider) => ({
  details: {},
  usability: {
    usable: true,
    status: "ready",
    reason_code: null,
    reason: null,
    blocking_provider_ids: [],
    recommended_action: "",
  },
  ...provider,
}));
const buildProviderOptions = (providerId) => ({
  provider_id: providerId,
  workspace_id: workspaceId,
  supports_load: false,
  auth_required: false,
  has_active_auth: true,
  auth_mode: "none",
  probed_at: "2025-01-01T00:00:00Z",
  models: {
    current_model_id: "fake-model",
    models: [{ id: "fake-model", name: "fake-model" }],
  },
});
const providerOptions = Object.fromEntries(
  normalizedProviders.map((provider) => [
    provider.provider_id,
    buildProviderOptions(provider.provider_id),
  ]),
);
const providersBootstrap = {
  providers: normalizedProviders,
  provider_options: providerOptions,
  provider_harness_config: {},
  codex_accounts: { active_account_id: null, accounts: [] },
  claude_accounts: { active_account_id: null, accounts: [] },
  gemini_accounts: { active_account_id: null, accounts: [] },
  qwen_accounts: { active_account_id: null, accounts: [] },
  kimi_accounts: { active_account_id: null, accounts: [] },
  mistral_accounts: { active_account_id: null, accounts: [] },
  copilot_accounts: { active_account_id: null, accounts: [] },
  cursor_accounts: { active_account_id: null, accounts: [] },
  amp_accounts: { active_account_id: null, accounts: [] },
  auggie_accounts: { active_account_id: null, accounts: [] },
};
const hasSnapshotEvent = streamEvents.some((item) => {
  const event = item?.event;
  return Boolean(
    event?.type === "snapshot" || event?.snapshot || event?.active_snapshot || event?.activeSnapshot,
  );
});
if (!hasSnapshotEvent && fixture.active_snapshot) {
  streamEvents = [
    {
      delay_ms: 0,
      event: {
        type: "snapshot",
        snapshot: fixture.active_snapshot,
        active_heads: activeHeadsBatch,
      },
    },
    ...streamEvents,
  ];
}

const baseDeltaEvent = streamEvents.find((item) => item?.event?.type === "session_head_delta")?.event ?? null;
const baseDelta = baseDeltaEvent?.delta ?? {};
const baseTurn = baseDelta?.turn ?? null;
const baseMessage = baseDelta?.message ?? null;
const baseSnapshotRev =
  baseDeltaEvent?.snapshot_rev ??
  fixture.active_snapshot?.snapshot_rev ??
  0;
const baseDelayMs = streamEvents.reduce((acc, item) => Math.max(acc, Number(item?.delay_ms ?? 0)), 0);
const replayMarkerPlans = [];

const resolveSessionIds = () => {
  if (Array.isArray(synthesizeSessionIds) && synthesizeSessionIds.length > 0) return synthesizeSessionIds;
  const fromSnapshot = Object.keys(snapshotBySession || {});
  if (fromSnapshot.length > 0) return fromSnapshot;
  const fallback = typeof baseDelta?.session_id === "string" ? [baseDelta.session_id] : [];
  return fallback;
};

const buildMessageContent = (sessionId, seq) => {
  const base = `Load test reply ${sessionId} ${seq}`;
  if (!Number.isFinite(synthesizeMessageBytes) || synthesizeMessageBytes <= base.length) {
    return base;
  }
  return `${base}${".".repeat(Math.max(0, synthesizeMessageBytes - base.length))}`;
};

const buildSummaryPreview = (sessionId, seq) => {
  const base = `Load test summary ${sessionId} ${seq}`;
  if (!Number.isFinite(synthesizeSummaryMessageBytes) || synthesizeSummaryMessageBytes <= base.length) {
    return base;
  }
  return `${base}${".".repeat(Math.max(0, synthesizeSummaryMessageBytes - base.length))}`;
};

const readSessionMeta = (sessionId) => {
  const snapshot = snapshotBySession?.[sessionId];
  const summary = snapshot?.summary ?? {};
  const head = snapshot?.head ?? {};
  const session = summary?.session ?? head?.session ?? {};
  const taskId = session?.task_id ?? baseMessage?.task_id ?? baseDelta?.task_id ?? "";
  const lastSeq =
    (typeof summary?.last_event_seq === "number" ? summary.last_event_seq : null) ??
    (typeof head?.last_event_seq === "number" ? head.last_event_seq : null) ??
    (typeof baseDelta?.last_event_seq === "number" ? baseDelta.last_event_seq : null) ??
    0;
  return { taskId, lastSeq };
};

const taskById = new Map(
  activeTasks
    .map((entry) => entry?.task)
    .filter(Boolean)
    .map((task) => [task.id, task]),
);

const sessionMetaById = new Map();

const getSessionMeta = (sessionId) => {
  const existing = sessionMetaById.get(sessionId);
  if (existing) return existing;
  const created = readSessionMeta(sessionId);
  sessionMetaById.set(sessionId, created);
  return created;
};

const getSessionHeadTurn = (sessionId) => {
  const turns = snapshotBySession?.[sessionId]?.head?.turns;
  if (!Array.isArray(turns) || turns.length === 0) return null;
  const turn = turns[turns.length - 1];
  return turn && typeof turn === "object" ? turn : null;
};

const getForegroundSessionId = () =>
  synthesizeForegroundTerminalSessionId ||
  fixture.active_snapshot?.active?.tasks?.[0]?.primary_session?.session?.id ||
  fixture.active_snapshot?.active?.tasks?.[0]?.task?.primary_session_id ||
  resolveSessionIds()[0] ||
  null;

let trackedForegroundFinal = null;

const registerForegroundFinalMarker = ({ sessionId, taskId, turnId, content, delayMs, lastSeq }) => {
  const markerId = `foreground-final:${sessionId}:${turnId}:${lastSeq ?? "na"}`;
  replayMarkerPlans.push({
    kind: "foreground_final",
    marker_id: markerId,
    session_id: sessionId,
    task_id: taskId,
    turn_id: turnId,
    content,
    delay_ms: delayMs,
    last_event_seq: lastSeq,
  });
  trackedForegroundFinal = {
    markerId,
    sessionId,
    taskId,
    turnId,
    content,
    lastSeq,
  };
};

const overwriteObject = (target, next) => {
  if (!target || !next || typeof target !== "object" || typeof next !== "object") return;
  for (const key of Object.keys(target)) {
    delete target[key];
  }
  Object.assign(target, JSON.parse(JSON.stringify(next)));
};

if (resetSessionHeadToRunningId) {
  const sessionId = resetSessionHeadToRunningId;
  const snapshot = snapshotBySession?.[sessionId];
  const activeTaskEntry = activeTasks.find(
    (entry) =>
      entry?.primary_session?.session?.id === sessionId ||
      entry?.task?.primary_session_id === sessionId,
  );
  const runningSummary = activeTaskEntry?.primary_session ?? null;
  const runningHead = activeTaskEntry?.primary_session_head ?? null;
  if (!snapshot || !runningSummary || !runningHead) {
    throw new Error(`Unable to reset session snapshot to running for ${sessionId}.`);
  }
  overwriteObject(snapshot.summary, runningSummary);
  overwriteObject(snapshot.head, runningHead);
  const activeHead = activeHeads.find((head) => head?.session?.id === sessionId);
  if (activeHead) {
    overwriteObject(activeHead, runningHead);
  }
  streamEvents = streamEvents.filter((item) => {
    const event = item?.event;
    const delta = event?.delta;
    if (event?.type !== "session_head_delta") return true;
    return delta?.session_id !== sessionId;
  });
}

let waitForTargets = [];
if (Number.isFinite(synthesizeDeltas) && synthesizeDeltas > 0) {
  const sessionIds = resolveSessionIds();
  if (sessionIds.length === 0) {
    throw new Error("No session ids available for synthesized deltas.");
  }
  const startDelay = Math.max(0, baseDelayMs + (Number.isFinite(synthesizeStartDelayMs) ? synthesizeStartDelayMs : 0));
  const interval = Math.max(0, Number.isFinite(synthesizeIntervalMs) ? synthesizeIntervalMs : 0);
  for (let i = 0; i < synthesizeDeltas; i++) {
    const sessionId = sessionIds[i % sessionIds.length];
    const meta = getSessionMeta(sessionId);
    const seq = meta.lastSeq + 1;
    meta.lastSeq = seq;
    sessionMetaById.set(sessionId, meta);
    const turnId = `turn-${sessionId}-${seq}`;
    const turn = {
      ...(baseTurn && typeof baseTurn === "object" ? baseTurn : {}),
      turn_id: turnId,
      session_id: sessionId,
      user_message_id: `msg-${sessionId}-user-${seq}`,
      status: baseTurn?.status ?? "completed",
      start_seq: seq,
      end_seq: seq,
      started_at: baseTurn?.started_at ?? new Date().toISOString(),
      updated_at: baseTurn?.updated_at ?? new Date().toISOString(),
      assistant_partial: null,
      thought_partial: null,
    };
    const message = {
      ...(baseMessage && typeof baseMessage === "object" ? baseMessage : {}),
      id: `msg-${sessionId}-assistant-${seq}`,
      session_id: sessionId,
      task_id: meta.taskId,
      turn_id: turnId,
      role: baseMessage?.role ?? "assistant",
      content: buildMessageContent(sessionId, seq),
      delivery: baseMessage?.delivery ?? "immediate",
      created_at: baseMessage?.created_at ?? new Date().toISOString(),
      order_seq: seq,
      turn_sequence: seq,
    };
    streamEvents.push({
      delay_ms: startDelay + i * interval,
      event: {
        type: "session_head_delta",
        workspace_id: workspaceId,
        snapshot_rev: baseSnapshotRev,
        delta: {
          session_id: sessionId,
          last_event_seq: seq,
          turn,
          message,
        },
      },
    });
  }

  if (!skipWaitForSynth) {
    waitForTargets = sessionIds
      .map((sessionId) => ({
        sessionId,
        lastSeq: sessionMetaById.get(sessionId)?.lastSeq ?? null,
      }))
      .filter((target) => Number.isFinite(target.lastSeq));
    if (!Number.isFinite(waitTimeoutMs)) {
      waitTimeoutMs = startDelay + synthesizeDeltas * interval + 10000;
    }
  }
}

if (Number.isFinite(synthesizeSummaryDeltas) && synthesizeSummaryDeltas > 0) {
  const foregroundSessionId = getForegroundSessionId();
  const requestedSessionIds =
    Array.isArray(synthesizeSummarySessionIds) && synthesizeSummarySessionIds.length > 0
      ? synthesizeSummarySessionIds
      : resolveSessionIds().filter((sessionId) => sessionId !== foregroundSessionId);
  const sessionIds = requestedSessionIds.length > 0 ? requestedSessionIds : resolveSessionIds();
  if (sessionIds.length === 0) {
    throw new Error("No session ids available for synthesized summary deltas.");
  }
  const startDelay =
    Math.max(0, baseDelayMs + (Number.isFinite(synthesizeSummaryStartDelayMs) ? synthesizeSummaryStartDelayMs : 0));
  const interval = Math.max(0, Number.isFinite(synthesizeSummaryIntervalMs) ? synthesizeSummaryIntervalMs : 0);
  for (let i = 0; i < synthesizeSummaryDeltas; i++) {
    const sessionId = sessionIds[i % sessionIds.length];
    const meta = getSessionMeta(sessionId);
    const seq = meta.lastSeq + 1;
    meta.lastSeq = seq;
    sessionMetaById.set(sessionId, meta);
    streamEvents.push({
      delay_ms: startDelay + i * interval,
      event: {
        type: "session_summary_delta",
        workspace_id: workspaceId,
        snapshot_rev: baseSnapshotRev,
        delta: {
          session_id: sessionId,
          task_id: meta.taskId,
          activity: {
            is_working: i % 2 === 0,
            last_turn_status: i % 2 === 0 ? "running" : "completed",
          },
          last_message_at: new Date().toISOString(),
          last_message_preview: buildSummaryPreview(sessionId, seq),
          last_event_seq: seq,
          projection_rev: seq,
          state_rev: seq,
        },
      },
    });
  }
  if (!Number.isFinite(waitTimeoutMs)) {
    waitTimeoutMs = startDelay + synthesizeSummaryDeltas * interval + 10000;
  }
}

if (Number.isFinite(synthesizeTaskDeltas) && synthesizeTaskDeltas > 0) {
  const foregroundSessionId = getForegroundSessionId();
  const requestedTaskIds =
    Array.isArray(synthesizeTaskIds) && synthesizeTaskIds.length > 0
      ? synthesizeTaskIds
      : activeTasks
          .filter((entry) => entry?.task?.id && entry?.task?.primary_session_id !== foregroundSessionId)
          .map((entry) => entry.task.id);
  const taskIds =
    requestedTaskIds.length > 0
      ? requestedTaskIds
      : activeTasks.map((entry) => entry?.task?.id).filter(Boolean);
  if (taskIds.length === 0) {
    throw new Error("No task ids available for synthesized task deltas.");
  }
  const startDelay =
    Math.max(0, baseDelayMs + (Number.isFinite(synthesizeTaskStartDelayMs) ? synthesizeTaskStartDelayMs : 0));
  const interval = Math.max(0, Number.isFinite(synthesizeTaskIntervalMs) ? synthesizeTaskIntervalMs : 0);
  for (let i = 0; i < synthesizeTaskDeltas; i++) {
    const taskId = taskIds[i % taskIds.length];
    const sourceTask = taskById.get(taskId);
    if (!sourceTask) continue;
    const timestamp = new Date(Date.now() + i * 1000).toISOString();
    streamEvents.push({
      delay_ms: startDelay + i * interval,
      event: {
        type: "task_delta",
        workspace_id: workspaceId,
        snapshot_rev: baseSnapshotRev,
        delta: {
          kind: "updated",
          task: {
            ...sourceTask,
            status: i % 2 === 0 ? "running" : sourceTask.status,
            updated_at: timestamp,
            last_activity_at: timestamp,
          },
        },
      },
    });
  }
  if (!Number.isFinite(waitTimeoutMs)) {
    waitTimeoutMs = startDelay + synthesizeTaskDeltas * interval + 10000;
  }
}

if (
  Number.isFinite(synthesizeForegroundTerminalDelayMs) &&
  synthesizeForegroundTerminalDelayMs >= 0
) {
  const sessionId = getForegroundSessionId();
  if (!sessionId) {
    throw new Error("No foreground session id available for synthesized terminal delta.");
  }
  const meta = getSessionMeta(sessionId);
  const seq = meta.lastSeq + 1;
  meta.lastSeq = seq;
  sessionMetaById.set(sessionId, meta);
  const existingTurn = getSessionHeadTurn(sessionId);
  const turnId =
    typeof existingTurn?.turn_id === "string" && existingTurn.turn_id.length > 0
      ? existingTurn.turn_id
      : `turn-${sessionId}-${seq}`;
  const userMessageId =
    typeof existingTurn?.user_message_id === "string" && existingTurn.user_message_id.length > 0
      ? existingTurn.user_message_id
      : `msg-${sessionId}-user-${seq}`;
  const content = `Foreground final ${sessionId} probe ${seq}`;
  const delayMs = Math.max(0, baseDelayMs + synthesizeForegroundTerminalDelayMs);
  streamEvents.push({
    delay_ms: delayMs,
    event: {
      type: "session_head_delta",
      workspace_id: workspaceId,
      snapshot_rev: baseSnapshotRev,
      delta: {
        session_id: sessionId,
        last_event_seq: seq,
        projection_rev: seq,
        state_rev: seq,
        turn: {
          ...(baseTurn && typeof baseTurn === "object" ? baseTurn : {}),
          ...(existingTurn && typeof existingTurn === "object" ? existingTurn : {}),
          turn_id: turnId,
          session_id: sessionId,
          user_message_id: userMessageId,
          status: "completed",
          start_seq:
            typeof existingTurn?.start_seq === "number" ? existingTurn.start_seq : seq,
          end_seq: seq,
          started_at:
            existingTurn?.started_at ??
            baseTurn?.started_at ??
            new Date().toISOString(),
          updated_at: new Date().toISOString(),
          assistant_partial: null,
          thought_partial: null,
        },
        event: {
          seq,
          id: `event-${sessionId}-${seq}`,
          session_id: sessionId,
          turn_id: turnId,
          event_type: "assistant_complete",
          payload_json: { full_content: content },
          created_at: new Date().toISOString(),
        },
        message: {
          ...(baseMessage && typeof baseMessage === "object" ? baseMessage : {}),
          id: `msg-${sessionId}-assistant-${seq}`,
          session_id: sessionId,
          task_id: meta.taskId,
          turn_id: turnId,
          role: "assistant",
          content,
          delivery: "immediate",
          created_at: new Date().toISOString(),
          order_seq: seq,
          turn_sequence: seq,
        },
      },
    },
  });
  registerForegroundFinalMarker({ sessionId, taskId: meta.taskId, turnId, content, delayMs, lastSeq: seq });
  waitForTargets = waitForTargets
    .filter((target) => target.sessionId !== sessionId)
    .concat([{ sessionId, lastSeq: seq }]);
  if (!Number.isFinite(waitTimeoutMs)) {
    waitTimeoutMs = delayMs + 10000;
  }
}

if (!trackedForegroundFinal) {
  const foregroundSessionId = getForegroundSessionId();
  if (foregroundSessionId) {
    const fallbackFinal = [...streamEvents]
      .sort((a, b) => Number(a?.delay_ms ?? 0) - Number(b?.delay_ms ?? 0))
      .reverse()
      .find((item) => {
        const delta = item?.event?.delta;
        const message = delta?.message;
        return (
          item?.event?.type === "session_head_delta" &&
          delta?.session_id === foregroundSessionId &&
          typeof message?.content === "string" &&
          message.content.length > 0 &&
          typeof message?.turn_id === "string"
        );
      });
    if (fallbackFinal) {
      const delta = fallbackFinal.event.delta;
      registerForegroundFinalMarker({
        sessionId: delta.session_id,
        taskId: delta.message.task_id ?? getSessionMeta(delta.session_id)?.taskId ?? "",
        turnId: delta.message.turn_id,
        content: delta.message.content,
        delayMs: Number(fallbackFinal.delay_ms ?? 0),
        lastSeq: delta.last_event_seq ?? null,
      });
    }
  }
}

const respondJson = async (route, body, status = 200) => {
  await route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body ?? null),
  });
};

const buildWorkerAppend = (events) => {
  const payload = JSON.stringify(events ?? []);
  const safePayload = JSON.stringify(payload).replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029");
  return `
;(() => {
  const __CTX_LOAD_TEST_EVENTS__ = JSON.parse(${safePayload});
  self.__CTX_LOAD_TEST__ = true;
  self.__CTX_LOAD_TEST_EVENTS__ = __CTX_LOAD_TEST_EVENTS__;

  const OriginalWebSocket = self.WebSocket;
  const matchesReplay = (url) => {
    const u = String(url ?? "");
    return u.includes("/api/workspaces/") && u.includes("/active_snapshot/stream");
  };

  class ReplayWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;

    constructor(url, protocols) {
      if (!matchesReplay(url)) {
        return new OriginalWebSocket(url, protocols);
      }
      this.url = String(url ?? "");
      this.readyState = ReplayWebSocket.CONNECTING;
      this.protocol = "";
      this.extensions = "";
      this.binaryType = "blob";
      this.bufferedAmount = 0;
      this.onopen = null;
      this.onmessage = null;
      this.onerror = null;
      this.onclose = null;
      this._listeners = new Map();
      this._timers = [];
      this._closed = false;

      const openTimer = self.setTimeout(() => {
        if (this._closed) return;
        this.readyState = ReplayWebSocket.OPEN;
        this._emit("open");
        this._startReplay();
      }, 0);
      this._timers.push(openTimer);
    }

    addEventListener(type, listener) {
      const list = this._listeners.get(type) || [];
      list.push(listener);
      this._listeners.set(type, list);
    }

    removeEventListener(type, listener) {
      const list = this._listeners.get(type);
      if (!list) return;
      const next = list.filter((item) => item !== listener);
      if (next.length === 0) {
        this._listeners.delete(type);
      } else {
        this._listeners.set(type, next);
      }
    }

    dispatchEvent(event) {
      const list = this._listeners.get(event.type) || [];
      for (const listener of list) {
        if (typeof listener === "function") {
          listener.call(this, event);
        } else if (listener && typeof listener.handleEvent === "function") {
          listener.handleEvent.call(listener, event);
        }
      }
      return true;
    }

    send() {
      // Ignore client messages; replay is one-way.
    }

    close() {
      if (this._closed) return;
      this.readyState = ReplayWebSocket.CLOSED;
      this._closed = true;
      for (const timer of this._timers) {
        self.clearTimeout(timer);
      }
      this._timers = [];
      this._emit("close");
    }

    _emit(type, data) {
      let evt;
      if (type === "message") {
        if (typeof MessageEvent !== "undefined") {
          evt = new MessageEvent("message", { data });
        } else {
          evt = new Event("message");
          evt.data = data;
        }
      } else if (type === "close") {
        if (typeof CloseEvent !== "undefined") {
          evt = new CloseEvent("close", { code: 1000, reason: "replay complete", wasClean: true });
        } else {
          evt = new Event("close");
        }
      } else {
        evt = new Event(type);
      }

      const handler = this[\`on\${type}\`];
      if (typeof handler === "function") {
        handler.call(this, evt);
      }
      this.dispatchEvent(evt);
    }

    _startReplay() {
      const events = self.__CTX_LOAD_TEST_EVENTS__ || [];
      for (const item of events) {
        const delay = Math.max(0, Number(item?.delay_ms ?? 0));
        const timer = self.setTimeout(() => {
          if (this._closed || this.readyState !== ReplayWebSocket.OPEN) return;
          this._emit("message", JSON.stringify(item.event));
        }, delay);
        this._timers.push(timer);
      }
    }
  }

  self.WebSocket = ReplayWebSocket;
})();
`;
};

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await context.newPage();
const debug = process.env.CTX_LOADTEST_DEBUG === "1";
if (debug) {
  page.on("console", (msg) => {
    if (msg.type() === "error") {
      console.log("page console error:", msg.text());
    }
  });
  page.on("requestfailed", (req) => {
    const failure = req.failure();
    console.log("page request failed:", req.url(), failure?.errorText ?? "unknown");
  });
  page.on("pageerror", (err) => {
    console.log("page error:", err?.message ?? String(err));
  });
}

await page.addInitScript(({ events, markerPlans }) => {
  window.__CTX_LOAD_TEST__ = true;
  window.__CTX_LOAD_TEST_EVENTS__ = Array.isArray(events) ? events : [];
  window.__ctxLoadTestReplay = {
    markerPlans: Array.isArray(markerPlans) ? markerPlans : [],
    markerEvents: [],
  };
  const nowMs = () => (performance.timeOrigin ?? Date.now()) + performance.now();
  for (const marker of window.__ctxLoadTestReplay.markerPlans) {
    const delay = Math.max(0, Number(marker?.delay_ms ?? 0));
    window.setTimeout(() => {
      window.__ctxLoadTestReplay.markerEvents.push({
        ...marker,
        fired_at_ms: nowMs(),
      });
    }, delay);
  }

  const OriginalWebSocket = window.WebSocket;
  const matchesReplay = (url) => {
    const u = String(url ?? "");
    return u.includes("/api/workspaces/") && u.includes("/active_snapshot/stream");
  };

  class ReplayWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;

    constructor(url, protocols) {
      if (!matchesReplay(url)) {
        return new OriginalWebSocket(url, protocols);
      }
      this.url = String(url ?? "");
      this.readyState = ReplayWebSocket.CONNECTING;
      this.protocol = "";
      this.extensions = "";
      this.binaryType = "blob";
      this.bufferedAmount = 0;
      this.onopen = null;
      this.onmessage = null;
      this.onerror = null;
      this.onclose = null;
      this._listeners = new Map();
      this._timers = [];
      this._closed = false;

      const openTimer = window.setTimeout(() => {
        if (this._closed) return;
        this.readyState = ReplayWebSocket.OPEN;
        this._emit("open");
        this._startReplay();
      }, 0);
      this._timers.push(openTimer);
    }

    addEventListener(type, listener) {
      const list = this._listeners.get(type) || [];
      list.push(listener);
      this._listeners.set(type, list);
    }

    removeEventListener(type, listener) {
      const list = this._listeners.get(type);
      if (!list) return;
      const next = list.filter((item) => item !== listener);
      if (next.length === 0) {
        this._listeners.delete(type);
      } else {
        this._listeners.set(type, next);
      }
    }

    dispatchEvent(event) {
      const list = this._listeners.get(event.type) || [];
      for (const listener of list) {
        if (typeof listener === "function") {
          listener.call(this, event);
        } else if (listener && typeof listener.handleEvent === "function") {
          listener.handleEvent.call(listener, event);
        }
      }
      return true;
    }

    send() {
      // Ignore client messages; replay is one-way.
    }

    close() {
      if (this._closed) return;
      this.readyState = ReplayWebSocket.CLOSED;
      this._closed = true;
      for (const timer of this._timers) {
        window.clearTimeout(timer);
      }
      this._timers = [];
      this._emit("close");
    }

    _emit(type, data) {
      let evt;
      if (type === "message") {
        if (typeof MessageEvent !== "undefined") {
          evt = new MessageEvent("message", { data });
        } else {
          evt = new Event("message");
          evt.data = data;
        }
      } else if (type === "close") {
        if (typeof CloseEvent !== "undefined") {
          evt = new CloseEvent("close", { code: 1000, reason: "replay complete", wasClean: true });
        } else {
          evt = new Event("close");
        }
      } else {
        evt = new Event(type);
      }

      const handler = this[`on${type}`];
      if (typeof handler === "function") {
        handler.call(this, evt);
      }
      this.dispatchEvent(evt);
    }

    _startReplay() {
      const events = window.__CTX_LOAD_TEST_EVENTS__ || [];
      for (const item of events) {
        const delay = Math.max(0, Number(item?.delay_ms ?? 0));
        const timer = window.setTimeout(() => {
          if (this._closed || this.readyState !== ReplayWebSocket.OPEN) return;
          this._emit("message", JSON.stringify(item.event));
        }, delay);
        this._timers.push(timer);
      }
    }
  }

  window.WebSocket = ReplayWebSocket;
}, { events: streamEvents, markerPlans: replayMarkerPlans });

await page.route("**/*workspaceActiveSnapshot.worker*", async (route) => {
  if (debug) {
    console.log("worker route shim:", route.request().url());
  }
  const response = await route.fetch();
  const body = await response.text();
  await route.fulfill({
    status: response.status(),
    headers: response.headers(),
    contentType: "application/javascript",
    body: `${body}\n${buildWorkerAppend(streamEvents)}`,
  });
});

await page.route("**/api/**", async (route) => {
  const request = route.request();
  const url = new URL(request.url());
  const method = request.method().toUpperCase();
  const pathname = url.pathname;
  if (!pathname.startsWith("/api/")) {
    await route.continue();
    return;
  }

  if (method === "OPTIONS") {
    await route.fulfill({ status: 204 });
    return;
  }

  if (pathname === "/api/health") {
    await respondJson(route, fixture.health);
    return;
  }

  if (pathname === "/api/providers") {
    await respondJson(route, normalizedProviders);
    return;
  }

  if (pathname === "/api/settings") {
    await respondJson(route, { execution: { mode: "host" } });
    return;
  }

  if (pathname === "/api/title_generation/local/status") {
    await respondJson(route, { enabled: false, ready: false });
    return;
  }

  if (pathname === "/api/desktop/log") {
    await route.fulfill({ status: 204, body: "" });
    return;
  }

  if (pathname.startsWith("/api/updates/check")) {
    await respondJson(route, {
      channel: "stable",
      base_url: "https://example.com",
      current_version: "0.0.0-fixture",
      latest_version: "0.0.0-fixture",
      update_available: false,
    });
    return;
  }

  if (pathname === "/api/workspaces") {
    await respondJson(route, fixture.workspace ? [fixture.workspace] : []);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}`) {
    await respondJson(route, fixture.workspace);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}/active_snapshot`) {
    await respondJson(route, fixture.active_snapshot);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}/active_heads`) {
    await respondJson(route, activeHeadsBatch);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}/providers/bootstrap`) {
    await respondJson(route, providersBootstrap);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}/terminals`) {
    await respondJson(route, []);
    return;
  }

  const providerOptionsMatch = pathname.match(
    new RegExp(`^/api/workspaces/${workspaceId}/providers/([^/]+)/options$`),
  );
  if (providerOptionsMatch) {
    const providerId = decodeURIComponent(providerOptionsMatch[1] || "");
    await respondJson(route, providerOptions[providerId] ?? buildProviderOptions(providerId));
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}/archived_task_summaries`) {
    await respondJson(route, {
      workspace_id: workspaceId,
      archived_rev: fixture.active_snapshot?.archived_rev ?? 0,
      tasks: [],
      next_cursor: null,
      total_archived: 0,
    });
    return;
  }

  const sessionMatch = pathname.match(/^\/api\/sessions\/([^/]+)(?:\/(.*))?$/);
  if (sessionMatch) {
    const sessionId = sessionMatch[1];
    const tail = sessionMatch[2] || "";

    if (tail === "head") {
      const snapshot = snapshotBySession[sessionId];
      await respondJson(route, snapshot?.head ?? {
        session: {},
        turns: [],
        messages: [],
        events: [],
        last_event_seq: 0,
        state_rev: 0,
        has_more_turns: false,
      });
      return;
    }

    if (tail.startsWith("snapshot")) {
      const snapshot = snapshotBySession[sessionId];
      await respondJson(route, snapshot ?? { summary: {}, head: { session: {}, turns: [], messages: [], last_event_seq: 0 } });
      return;
    }

    if (tail.startsWith("state")) {
      const snapshot = snapshotBySession[sessionId];
      await respondJson(route, snapshot?.state ?? { artifacts: [], git_status: null });
      return;
    }

    if (tail.startsWith("artifacts")) {
      await respondJson(route, []);
      return;
    }

    if (tail.startsWith("subagent_invocations")) {
      await respondJson(route, []);
      return;
    }

    if (tail.startsWith("turns/") && tail.endsWith("/tools")) {
      await respondJson(route, []);
      return;
    }

    if (tail.startsWith("diff_summary")) {
      await respondJson(route, { summary: "", added: 0, removed: 0 });
      return;
    }

    if (tail.startsWith("diff")) {
      await respondJson(route, { diff: "" });
      return;
    }

    if (tail.startsWith("git_status")) {
      await respondJson(route, { summary_line: "", branch: null, ahead: 0, behind: 0, detached: false, staged: 0, unstaged: 0, untracked: 0, entries: [] });
      return;
    }
  }

  if (/^\/api\/tasks\/[^/]+\/mark_read$/.test(pathname)) {
    await route.fulfill({ status: 204, body: "" });
    return;
  }

  if (pathname === "/api/telemetry/client") {
    try {
      const body = route.request().postDataJSON?.() ?? JSON.parse(route.request().postData() || "{}");
      if (Array.isArray(body?.events)) {
        clientTelemetryEvents.push(...body.events);
      }
    } catch {
      // Ignore malformed telemetry payloads; they should not block the replay harness.
    }
    await route.fulfill({ status: 204 });
    return;
  }

  console.log("Fixture miss:", method, pathname);
  await route.fulfill({ status: 404, contentType: "application/json", body: JSON.stringify({ error: "fixture miss" }) });
});

try {
  const url = new URL(`${baseUrl}/workspaces/${workspaceId}`);
  url.searchParams.set("loadtest", "1");
  if (waitForTargets.length > 0 || trackedForegroundFinal) {
    url.searchParams.set("ctxE2E", "1");
  }
  await page.goto(url.toString(), { waitUntil: "domcontentloaded" });
  try {
  await page.waitForFunction(() => document.querySelectorAll(".wb-task-row").length >= 1, null, { timeout: 15000 });
  } catch (err) {
    const taskCount = await page.evaluate(() => document.querySelectorAll(".wb-task-row").length);
    console.log(`task rows after timeout: ${taskCount}`);
    if (debug) {
      const snapshotPath = path.join(os.tmpdir(), "ctx-web-loadtest-timeout.png");
      await page.screenshot({ path: snapshotPath, fullPage: true });
      console.log(`wrote timeout screenshot to ${snapshotPath}`);
    }
    throw err;
  }
  await page.evaluate(() => window.__ctxLoadTestTelemetry?.reset?.());

  const rows = page.locator(".wb-task-row");
  const taskCount = await rows.count();
  if (taskCount === 0) {
    throw new Error("Replay fixture did not render any task rows.");
  }
  if (trackedForegroundFinal?.taskId) {
    const trackedTaskTitle = taskById.get(trackedForegroundFinal.taskId)?.title ?? null;
    if (trackedTaskTitle) {
      await page.locator(".wb-task-row").filter({ hasText: trackedTaskTitle }).first().click();
    } else {
      await rows.first().click();
    }
    try {
      await page.locator(".wb-session-slot textarea.wb-active-textarea").waitFor({
        state: "visible",
        timeout: 10000,
      });
    } catch (error) {
      const shellDebug = await page.evaluate(() => ({
        active_textarea_count: document.querySelectorAll(".wb-session-slot textarea.wb-active-textarea").length,
        session_slot_count: document.querySelectorAll(".wb-session-slot").length,
        task_rows: Array.from(document.querySelectorAll(".wb-task-row")).map((row) => row.textContent?.trim() ?? ""),
      }));
      console.error(
        "foreground session composer never became visible:",
        JSON.stringify(
          {
            target: trackedForegroundFinal,
            shell_debug: shellDebug,
          },
          null,
          2,
        ),
      );
      throw error;
    }
  } else {
    const useDirectClicks = Number.isFinite(synthesizeDeltas) && synthesizeDeltas > 0;
    const clickSequence =
      taskCount >= 2
        ? [0, 1, 0]
        : [0];
    for (let i = 0; i < clickSequence.length; i += 1) {
      const targetIndex = clickSequence[i];
      if (useDirectClicks) {
        await page.evaluate((index) => document.querySelectorAll(".wb-task-row")[index]?.click(), targetIndex);
      } else {
        await rows.nth(targetIndex).click();
      }
      if (i < clickSequence.length - 1) {
        await page.waitForTimeout(i === 0 ? 300 : 400);
      }
    }
  }

  await page.waitForTimeout(1500);

  if (waitForTargets.length > 0) {
    await page.waitForFunction(
      ({ targets }) => {
        const getSeq = window.__ctxE2E?.getSessionLastEventSeq;
        if (typeof getSeq !== "function") return false;
        return targets.every((target) => {
          const current = getSeq(target.sessionId);
          return typeof current === "number" && current >= target.lastSeq;
        });
      },
      { targets: waitForTargets },
      { timeout: Number.isFinite(waitTimeoutMs) ? waitTimeoutMs : 30000 },
    );
  }

  let replayFinalToStateMs = [];
  let replayFinalToDomMs = [];
  if (trackedForegroundFinal) {
    const markerHandle = await page.waitForFunction(
      ({ markerId }) => {
        const entries = window.__ctxLoadTestReplay?.markerEvents ?? [];
        return entries.find((entry) => entry?.marker_id === markerId) ?? null;
      },
      { markerId: trackedForegroundFinal.markerId },
      { timeout: Number.isFinite(waitTimeoutMs) ? waitTimeoutMs : 30000 },
    );
    const marker = await markerHandle.jsonValue();
    let stateHandle;
    try {
      stateHandle = await page.waitForFunction(
        ({ sessionId, turnId, content }) => {
          const getMessages = window.__ctxE2E?.getSessionHeadMessages;
          if (typeof getMessages !== "function") return null;
          const messages = getMessages(sessionId);
          if (!Array.isArray(messages)) return null;
          const found = messages.some(
            (message) =>
              message === content ||
              (typeof message === "object" &&
                message !== null &&
                message?.turn_id === turnId &&
                message?.role === "assistant" &&
                message?.content === content),
          );
          if (found) {
            return (performance.timeOrigin ?? Date.now()) + performance.now();
          }
          return null;
        },
        {
          sessionId: trackedForegroundFinal.sessionId,
          turnId: trackedForegroundFinal.turnId,
          content: trackedForegroundFinal.content,
        },
        { timeout: Number.isFinite(waitTimeoutMs) ? waitTimeoutMs : 30000 },
      );
    } catch (error) {
      const debugMessages = await page.evaluate((sessionId) => {
        const getMessages = window.__ctxE2E?.getSessionHeadMessages;
        return typeof getMessages === "function" ? getMessages(sessionId) : null;
      }, trackedForegroundFinal.sessionId);
      console.error(
        "tracked foreground final never reached head messages:",
        JSON.stringify(
          {
            target: trackedForegroundFinal,
            messages: Array.isArray(debugMessages) ? debugMessages.slice(-5) : debugMessages,
          },
          null,
          2,
        ),
      );
      throw error;
    }
    const stateAtMs = await stateHandle.jsonValue();
    try {
      await page.waitForFunction(
        ({ content }) => document.body?.textContent?.includes(content) ?? false,
        { content: trackedForegroundFinal.content },
        { timeout: Number.isFinite(waitTimeoutMs) ? waitTimeoutMs : 30000 },
      );
    } catch (error) {
      const assistantEntries = await page.locator(".wb-assistant-entry").allInnerTexts();
      const debugState = await page.evaluate(({ sessionId, content }) => {
        const bridge = window.__ctxE2E;
        const visibleSlot = document.querySelector('.wb-session-slot[aria-hidden="false"]');
        const visibleText = visibleSlot?.textContent ?? "";
        const visibleEntry = bridge?.getVisibleSessionEntryDebug?.() ?? null;
        const visibleThread = bridge?.getVisibleSessionThreadDebug?.() ?? null;
        const headMessages = bridge?.getSessionHeadMessages?.(sessionId) ?? [];
        const headUserMessages = bridge?.getSessionHeadUserMessages?.(sessionId) ?? [];
        return {
          workspaceConnection: bridge?.getWorkspaceSnapshot?.()?.connection ?? null,
          visibleSlotSessionId: visibleSlot?.getAttribute("data-session-id") ?? null,
          visibleTextHasContent: visibleText.includes(content),
          visibleTextTail: visibleText.slice(-2000),
          visibleEntrySessionId: visibleEntry?.sessionId ?? null,
          visibleEntryHasContent:
            Array.isArray(visibleEntry?.messageContents) &&
            visibleEntry.messageContents.some((message) => String(message).includes(content)),
          visibleEntryLastEventSeq: visibleEntry?.lastEventSeq ?? null,
          visibleThreadSessionId: visibleThread?.sessionId ?? null,
          visibleThreadProjectionRev: visibleThread?.projectionRev ?? null,
          visibleThreadTurnsStamp: visibleThread?.turnsStamp ?? null,
          visibleThreadMessagesStamp: visibleThread?.messagesStamp ?? null,
          visibleThreadHasContent:
            Array.isArray(visibleThread?.assistantContents) &&
            visibleThread.assistantContents.some((message) => String(message).includes(content)),
          visibleThreadTail: Array.isArray(visibleThread?.assistantContents)
            ? visibleThread.assistantContents.slice(-3)
            : [],
          visibleThreadLastItemIds: Array.isArray(visibleThread?.listItemIds)
            ? visibleThread.listItemIds.slice(-12)
            : [],
          headHasContent: Array.isArray(headMessages) && headMessages.some((message) => String(message).includes(content)),
          headUserMessagesTail: Array.isArray(headUserMessages) ? headUserMessages.slice(-3) : [],
          headMessagesTail: Array.isArray(headMessages) ? headMessages.slice(-3) : [],
          pretextPerf: window.__ctxPretextPerfDiagnostics?.getSnapshot?.() ?? null,
        };
      }, {
        sessionId: trackedForegroundFinal.sessionId,
        content: trackedForegroundFinal.content,
      });
      console.error(
        "tracked foreground final never reached visible text:",
        JSON.stringify(
          {
            target: trackedForegroundFinal,
            assistant_entries: assistantEntries,
            debug_state: debugState,
          },
          null,
          2,
        ),
      );
      throw error;
    }
    const domAtMs = await page.evaluate(
      () => (performance.timeOrigin ?? Date.now()) + performance.now(),
    );
    if (Number.isFinite(marker?.fired_at_ms) && Number.isFinite(stateAtMs)) {
      replayFinalToStateMs = [stateAtMs - marker.fired_at_ms];
    }
    if (Number.isFinite(marker?.fired_at_ms) && Number.isFinite(domAtMs)) {
      replayFinalToDomMs = [domAtMs - marker.fired_at_ms];
    }
  }

  const telemetry = await page.evaluate(() => window.__ctxLoadTestTelemetry?.getSnapshot?.());
  const pretextPerf = await page.evaluate(() => window.__ctxPretextPerfDiagnostics?.getSnapshot?.() ?? null);
  if (check) {
    if (!Number.isFinite(maxSessionSwitchP95)) maxSessionSwitchP95 = 100;
    if (!Number.isFinite(maxSessionSwitchP99)) maxSessionSwitchP99 = 300;
    if (!Number.isFinite(maxLongTaskMs)) maxLongTaskMs = 50;
    if (!Number.isFinite(maxLongTaskCount)) maxLongTaskCount = 0;
  }

  const sessionDurations = Array.isArray(telemetry?.session_switches)
    ? telemetry.session_switches
        .filter((entry) => entry?.status === "completed" && Number.isFinite(entry?.duration_ms))
        .map((entry) => entry.duration_ms)
    : [];
  const longTasks = Array.isArray(telemetry?.long_tasks) ? telemetry.long_tasks : [];
  const longTaskDurations = longTasks
    .filter((entry) => Number.isFinite(entry?.duration_ms))
    .map((entry) => entry.duration_ms);
  const summarizeClientMetric = (name) => {
    const values = clientTelemetryEvents
      .filter((event) => event?.name === name && Number.isFinite(event?.value))
      .map((event) => event.value);
    return {
      count: values.length,
      p50: percentile(values, 0.5),
      p95: percentile(values, 0.95),
      p99: percentile(values, 0.99),
      max: values.length ? Math.max(...values) : null,
    };
  };
  const countClientCounterMetric = (name, labels = {}) =>
    clientTelemetryEvents.filter((event) => {
      if (event?.name !== name) return false;
      const eventLabels = event?.labels ?? {};
      return Object.entries(labels).every(([key, value]) => eventLabels?.[key] === value);
    }).length;
  const longTaskBudget = Number.isFinite(maxLongTaskMs) ? maxLongTaskMs : null;
  const longTasksOverBudget = longTaskBudget
    ? longTaskDurations.filter((value) => value > longTaskBudget).length
    : 0;
  const summary = {
    session_switch_ms: summarizeValues(sessionDurations),
    long_tasks_ms: {
      count: longTaskDurations.length,
      max: longTaskDurations.length ? Math.max(...longTaskDurations) : null,
      over_budget: longTasksOverBudget,
      budget_ms: longTaskBudget,
    },
    replay_final_to_state_ms: summarizeValues(replayFinalToStateMs),
    replay_final_to_dom_ms: summarizeValues(replayFinalToDomMs),
    final_ws_to_dom_ms: summarizeClientMetric("workbench.final_ws_to_dom_ms"),
    final_ingress_to_dom_ms: summarizeClientMetric("workbench.final_ingress_to_dom_ms"),
    interrupt_click_to_pending_ms: summarizeClientMetric("workbench.interrupt_click_to_pending_ms"),
    switch_to_first_paint_ms: summarizeClientMetric("workbench.switch_to_first_paint_ms"),
    switch_to_authoritative_ms: summarizeClientMetric("workbench.switch_to_authoritative_ms"),
    foreground_queue_age_ms: summarizeClientMetric("workbench.foreground_queue_age_ms"),
    workspace_backlog_age_ms: summarizeClientMetric("workbench.workspace_backlog_age_ms"),
    foreground_gap_recovery_ms: summarizeClientMetric("workbench.foreground_gap_recovery_ms"),
    stale_pending_after_terminal_assistant_message: countClientCounterMetric(
      "workbench.thread.contract_violation_count",
      { reason: "stale_pending_after_terminal_assistant_message" },
    ),
    stale_pending_duplicate_assistant_message: countClientCounterMetric(
      "workbench.thread.contract_violation_count",
      { reason: "stale_pending_duplicate_assistant_message" },
    ),
    late_chunk_after_terminal_count: countClientCounterMetric(
      "workbench.late_chunk_after_terminal_count",
    ),
    projection_or_seq_regression_count: countClientCounterMetric(
      "workbench.projection_or_seq_regression_count",
    ),
    gap_repair_mismatch_count: countClientCounterMetric(
      "workbench.gap_repair_mismatch_count",
    ),
    switch_stale_visible_count: countClientCounterMetric(
      "workbench.switch_stale_visible_count",
    ),
    nav_thread_activity_mismatch_count: countClientCounterMetric(
      "workbench.nav_thread_activity_mismatch_count",
    ),
    workspace_stream_reset_count: countClientCounterMetric(
      "workbench.workspace_stream_reset_count",
    ),
    foreground_rehydrate_count: countClientCounterMetric(
      "workbench.foreground_rehydrate_count",
    ),
  };
  console.log(
    `session switches: count=${summary.session_switch_ms.count} ` +
      `p50=${summary.session_switch_ms.p50 ?? "n/a"} ` +
      `p95=${summary.session_switch_ms.p95 ?? "n/a"} ` +
      `p99=${summary.session_switch_ms.p99 ?? "n/a"} ` +
      `max=${summary.session_switch_ms.max ?? "n/a"}`,
  );
  console.log(
    `long tasks: count=${summary.long_tasks_ms.count} ` +
      `over_budget=${summary.long_tasks_ms.over_budget} ` +
      `max=${summary.long_tasks_ms.max ?? "n/a"} ` +
      `budget=${summary.long_tasks_ms.budget_ms ?? "n/a"}`,
  );
  console.log(
    `replay final marker->state: count=${summary.replay_final_to_state_ms.count} ` +
      `p95=${summary.replay_final_to_state_ms.p95 ?? "n/a"} ` +
      `max=${summary.replay_final_to_state_ms.max ?? "n/a"}`,
  );
  console.log(
    `replay final marker->dom: count=${summary.replay_final_to_dom_ms.count} ` +
      `p95=${summary.replay_final_to_dom_ms.p95 ?? "n/a"} ` +
      `max=${summary.replay_final_to_dom_ms.max ?? "n/a"}`,
  );
  console.log(
    `final ws->dom: count=${summary.final_ws_to_dom_ms.count} ` +
      `p95=${summary.final_ws_to_dom_ms.p95 ?? "n/a"} ` +
      `max=${summary.final_ws_to_dom_ms.max ?? "n/a"}`,
  );
  console.log(
    `interrupt click->pending: count=${summary.interrupt_click_to_pending_ms.count} ` +
      `p95=${summary.interrupt_click_to_pending_ms.p95 ?? "n/a"} ` +
      `max=${summary.interrupt_click_to_pending_ms.max ?? "n/a"}`,
  );

  const failures = [];
  if (check && sessionDurations.length === 0) {
    failures.push("no session switch telemetry captured");
  }
  if (check && Number.isFinite(maxSessionSwitchP95)) {
    const p95 = summary.session_switch_ms.p95 ?? Infinity;
    if (p95 > maxSessionSwitchP95) {
      failures.push(`session switch p95 ${p95}ms > ${maxSessionSwitchP95}ms`);
    }
  }
  if (check && Number.isFinite(maxSessionSwitchP99)) {
    const p99 = summary.session_switch_ms.p99 ?? Infinity;
    if (p99 > maxSessionSwitchP99) {
      failures.push(`session switch p99 ${p99}ms > ${maxSessionSwitchP99}ms`);
    }
  }
  if (check && Number.isFinite(maxLongTaskCount)) {
    if (summary.long_tasks_ms.over_budget > maxLongTaskCount) {
      failures.push(
        `long tasks over budget ${summary.long_tasks_ms.over_budget} > ${maxLongTaskCount}`,
      );
    }
  }

  const output = {
    fixture: fixturePath,
    base_url: baseUrl,
    workspace_id: workspaceId,
    captured_at: new Date().toISOString(),
    telemetry,
    pretext_perf: pretextPerf,
    replay_markers: replayMarkerPlans,
    tracked_foreground_final: trackedForegroundFinal,
    client_telemetry: clientTelemetryEvents,
    summary,
  };
  await mkdir(path.dirname(outPath), { recursive: true });
  await writeFile(outPath, JSON.stringify(output, null, 2));
  console.log(`wrote telemetry to ${outPath}`);

  if (failures.length > 0) {
    console.error("loadtest gate failed:");
    for (const failure of failures) {
      console.error(`- ${failure}`);
    }
    process.exit(1);
  }
} finally {
  await browser.close();
}
