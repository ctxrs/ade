import fs from "fs/promises";
import type { APIRequestContext, Page, Request } from "playwright/test";
import { test, expect } from "./fixtures";
import { clearDiagnostics, getDiagnostics } from "./utils/diagnostics";
import {
  postImmediateMessageAndWaitForCompletion,
  seedDummyWorkspace,
} from "./utils/seedDummyWorkspace";

const ENABLED = process.env.CTX_REMOTE_DAEMON_STREAM_SOAK === "1";
const FAULT_MODE = process.env.CTX_REMOTE_DAEMON_STREAM_SOAK_FAULT ?? "";
const REMOTE_MODE =
  process.env.CTX_REMOTE_DAEMON_STREAM_SOAK_REMOTE === "1" ||
  Boolean(process.env.CTX_E2E_BASE_URL && !process.env.CTX_E2E_BASE_URL.includes("127.0.0.1"));

const envNumber = (name: string, fallback: number): number => {
  const raw = process.env[name];
  if (!raw) return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
};

const TASK_COUNT = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_TASKS", 16);
const TURNS_PER_SESSION = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_TURNS", 3);
const MESSAGE_BYTES = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_MESSAGE_BYTES", 1800);
const STREAMERS = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_STREAMERS", 6);
const STREAM_INTERVAL_MS = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_STREAM_INTERVAL_MS", 5);
const STREAM_TIMEOUT_MS = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_STREAM_TIMEOUT_MS", 75_000);
const MIN_STREAM_EVENTS = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_MIN_EVENTS", 2000);
const MIN_SESSION_HEAD_DELTAS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MIN_SESSION_HEAD_DELTAS",
  1000,
);
const PROBE_COUNT = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_PROBES", 4);
const PROBE_TIMEOUT_MS = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_PROBE_TIMEOUT_MS", 35_000);
const HEAD_POLL_MS = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_HEAD_POLL_MS", 75);
const CLOCK_SAMPLES = envNumber("CTX_REMOTE_DAEMON_STREAM_SOAK_CLOCK_SAMPLES", 7);
const MAX_CLOCK_UNCERTAINTY_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_CLOCK_UNCERTAINTY_MS",
  REMOTE_MODE ? 1500 : 750,
);

const MAX_VISIBLE_SILENCE_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_STALENESS_MS",
  REMOTE_MODE ? 8000 : 5000,
);
const MAX_BACKEND_TO_DOM_P95_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_BACKEND_TO_DOM_P95_MS",
  REMOTE_MODE ? 5000 : 2500,
);
const MAX_BACKEND_TO_DOM_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_BACKEND_TO_DOM_MS",
  REMOTE_MODE ? 10_000 : 5000,
);
const MAX_CLIENT_RECEIVE_LAG_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_CLIENT_RECEIVE_LAG_MS",
  10_000,
);
const MAX_REPLICA_APPLY_LAG_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_REPLICA_APPLY_LAG_MS",
  10_000,
);
const MAX_CLICK_TO_PENDING_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_CLICK_TO_PENDING_MS",
  REMOTE_MODE ? 1500 : 1000,
);
const MAX_CLICK_TO_TERMINAL_MS = envNumber(
  "CTX_REMOTE_DAEMON_STREAM_SOAK_MAX_CLICK_TO_TERMINAL_MS",
  REMOTE_MODE ? 25_000 : 15_000,
);

type SessionHeadResponse = {
  turns?: Array<{
    turn_id?: string;
    user_message_id?: string;
    status?: string;
  }>;
  messages?: Array<{
    id?: string;
    turn_id?: string;
    role?: string;
    content?: string;
  }>;
};

type TelemetryMetricSummary = {
  name?: string;
  count?: number;
  sum?: number;
  min?: number | null;
  max?: number | null;
  p50?: number | null;
  p95?: number | null;
  p99?: number | null;
  labels?: Record<string, string>;
};

type TelemetrySummaryResponse = {
  metrics?: TelemetryMetricSummary[];
  window_ms?: number | null;
};

type MetricRollup = {
  count: number;
  sum: number;
  min: number | null;
  max: number | null;
  p50: number | null;
  p95: number | null;
  p99: number | null;
};

type ProbeOutcome = {
  marker: string;
  turnId: string | null;
  backendReadyAtMs: number | null;
  domVisibleAtMs: number | null;
  backendToDomMs: number | null;
  timedOut: boolean;
  error: string | null;
};

type ClockSample = {
  startedAtMs: number;
  endedAtMs: number;
  rttMs: number;
  daemonUnixMs: number;
  offsetMs: number;
  uncertaintyMs: number;
};

type ClockCalibration = {
  samples: ClockSample[];
  offsetMs: number;
  uncertaintyMs: number;
  minRttMs: number;
  p95RttMs: number | null;
};

type VisibleProgressSnapshot = {
  startedAtMs: number;
  endedAtMs: number;
  samples: Array<{
    at_ms: number;
    text_length: number;
    signature: string;
  }>;
};

type BoundedWriter = {
  stop: () => Promise<void>;
  getStats: () => { sent: number; failures: string[]; backpressure: number };
};

type WorkspaceStreamTelemetrySample = {
  lane: "foreground" | "workspace";
  eventType: string;
  sessionId: string | null;
  emittedAtMs: number | null;
  receivedAtMs: number;
};

type RemoteDaemonLoadWindow = Window & {
  __ctxVisibleProgressProbe?: {
    getSnapshot: () => VisibleProgressSnapshot;
    stop: () => VisibleProgressSnapshot;
  };
  __ctxWorkspaceStreamTelemetrySamples?: WorkspaceStreamTelemetrySample[];
};

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

const percentile = (values: number[], p: number): number | null => {
  if (values.length === 0) return null;
  const sorted = values.slice().sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * p) - 1));
  return Math.round(sorted[index]! * 10) / 10;
};

const metricRollupEmpty = (): MetricRollup => ({
  count: 0,
  sum: 0,
  min: null,
  max: null,
  p50: null,
  p95: null,
  p99: null,
});

const formatUnknownError = (error: unknown): string => {
  if (error instanceof Error && error.message) return error.message;
  return String(error);
};

const buildSlowPrompt = (
  marker: string,
  opts?: { toolCount?: number; bodyLines?: number },
): string => {
  const toolCount = opts?.toolCount ?? 5;
  const bodyLines = opts?.bodyLines ?? 80;
  const tools = Array.from({ length: toolCount }, (_, index) => ({
    kind: "execute",
    title: `remote stream tool ${index + 1}`,
    input: { command: `printf '${marker}-${index + 1}'` },
    output_text: `${marker} output ${index + 1}`,
  }));
  const body = Array.from(
    { length: bodyLines },
    (_, index) => `remote daemon stream visible progress ${marker} line ${index + 1}`,
  ).join("\n");
  return `slow-diff-test stream-assistant-partials emit-thought ${marker}
${body}
[[tool_calls]]
${JSON.stringify(tools)}
[[/tool_calls]]`;
};

const buildBackgroundPrompt = (label: string): string => {
  const tools = Array.from({ length: 4 }, (_, index) => ({
    kind: "execute",
    title: `background stream tool ${index + 1}`,
    input: { command: `printf '${label}-${index + 1}'` },
    output_text: `${label} output ${index + 1}`,
  }));
  const base = `remote-daemon-load-background ${label}`;
  const padding = MESSAGE_BYTES > base.length ? `\n${"x".repeat(MESSAGE_BYTES - base.length)}` : "";
  return `${base}${padding}
[[tool_calls]]
${JSON.stringify(tools)}
[[/tool_calls]]`;
};

function startBoundedBackgroundWriters(
  request: APIRequestContext,
  sessionIds: readonly string[],
): BoundedWriter[] {
  return sessionIds.map((sessionId, sessionIndex) => {
    let stopped = false;
    let sent = 0;
    let backpressure = 0;
    const failures: string[] = [];
    const loop = (async () => {
      while (!stopped) {
        const label = `s${sessionIndex + 1}-${sent + 1}-${Date.now()}`;
        try {
          await postImmediateMessageAndWaitForCompletion(
            request,
            sessionId,
            buildBackgroundPrompt(label),
            { timeoutMs: 30_000 },
          );
          sent += 1;
        } catch (error) {
          const message = formatUnknownError(error);
          if (
            message.includes("A turn is already running") ||
            message.includes("turn completion timeout")
          ) {
            backpressure += 1;
          } else {
            failures.push(message);
          }
          await sleep(250);
        }
        await sleep(Math.max(STREAM_INTERVAL_MS, 100));
      }
    })();
    return {
      stop: async () => {
        stopped = true;
        await loop;
      },
      getStats: () => ({ sent, failures: failures.slice(), backpressure }),
    };
  });
}

async function sendSessionMessage(
  request: APIRequestContext,
  sessionId: string,
  content: string,
): Promise<string> {
  const response = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content,
      delivery: "immediate",
    },
  });
  if (!response.ok()) {
    const body = await response.text().catch(() => "");
    const suffix = body.trim() ? `: ${body.trim()}` : "";
    throw new Error(`message send failed: ${response.url()} (${response.status()})${suffix}`);
  }
  const payload = (await response.json()) as { id?: string };
  return String(payload.id ?? "");
}

async function waitForForegroundTurnCompletion(
  request: APIRequestContext,
  sessionId: string,
  marker: string,
  timeoutMs: number,
): Promise<{ backendReadyAtMs: number; turnId: string }> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const response = await request.get(`/api/sessions/${sessionId}/head`);
    expect(response.ok(), `head request failed: ${response.url()}`).toBeTruthy();
    const payload = (await response.json()) as SessionHeadResponse;
    const messages = Array.isArray(payload.messages) ? payload.messages : [];
    const turns = Array.isArray(payload.turns) ? payload.turns : [];
    const userMessage = messages.find(
      (message) =>
        message?.role === "user" &&
        typeof message.content === "string" &&
        message.content.includes(marker),
    );
    if (userMessage?.id) {
      const turn = turns.find((entry) => entry?.user_message_id === userMessage.id);
      const assistantMessage = messages.find(
        (message) =>
          message?.turn_id === turn?.turn_id &&
          message?.role === "assistant" &&
          typeof message.content === "string" &&
          message.content.includes(marker),
      );
      const status = String(turn?.status ?? "").toLowerCase();
      if (assistantMessage?.content && (status === "completed" || status === "done")) {
        return {
          backendReadyAtMs: Date.now(),
          turnId: String(turn?.turn_id ?? ""),
        };
      }
    }
    await sleep(HEAD_POLL_MS);
  }
  throw new Error(`foreground turn did not complete for marker ${marker}`);
}

async function waitForVisibleMarker(
  page: Page,
  marker: string,
  timeoutMs: number,
): Promise<number> {
  const handle = await page.waitForFunction(
    ({ text }) => {
      const session = document.querySelector('.wb-session-slot[aria-hidden="false"]');
      return session?.textContent?.includes(text) ? Date.now() : null;
    },
    { text: marker },
    { timeout: timeoutMs },
  );
  const value = await handle.jsonValue();
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`marker ${marker} did not expose a visible timestamp`);
  }
  return value;
}

async function runForegroundProbe(
  page: Page,
  request: APIRequestContext,
  sessionId: string,
  marker: string,
): Promise<ProbeOutcome> {
  try {
    await sendSessionMessage(
      request,
      sessionId,
      buildSlowPrompt(marker, { bodyLines: 24, toolCount: 2 }),
    );
    const { backendReadyAtMs, turnId } = await waitForForegroundTurnCompletion(
      request,
      sessionId,
      marker,
      PROBE_TIMEOUT_MS,
    );
    const domVisibleAtMs = await waitForVisibleMarker(page, marker, PROBE_TIMEOUT_MS);
    return {
      marker,
      turnId,
      backendReadyAtMs,
      domVisibleAtMs,
      backendToDomMs: domVisibleAtMs - backendReadyAtMs,
      timedOut: false,
      error: null,
    };
  } catch (error) {
    return {
      marker,
      turnId: null,
      backendReadyAtMs: null,
      domVisibleAtMs: null,
      backendToDomMs: null,
      timedOut: true,
      error: formatUnknownError(error),
    };
  }
}

async function waitForTerminalInterrupted(
  request: APIRequestContext,
  sessionId: string,
  marker: string,
  timeoutMs: number,
): Promise<{ terminalAtMs: number | null; status: string | null }> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const response = await request.get(`/api/sessions/${sessionId}/head`);
    expect(response.ok(), `head request failed: ${response.url()}`).toBeTruthy();
    const payload = (await response.json()) as SessionHeadResponse;
    const messages = Array.isArray(payload.messages) ? payload.messages : [];
    const turns = Array.isArray(payload.turns) ? payload.turns : [];
    const userMessage = messages.find(
      (message) =>
        message?.role === "user" &&
        typeof message.content === "string" &&
        message.content.includes(marker),
    );
    const turn = turns.find((entry) => entry?.user_message_id === userMessage?.id);
    const status = String(turn?.status ?? "").toLowerCase();
    if (status === "interrupted" || status === "cancelled" || status === "canceled") {
      return { terminalAtMs: Date.now(), status };
    }
    await sleep(HEAD_POLL_MS);
  }
  return { terminalAtMs: null, status: null };
}

async function readTelemetryMetric(
  request: APIRequestContext,
  metric: string,
  windowMs: number,
): Promise<MetricRollup> {
  const response = await request.get(
    `/api/telemetry/summary?metric=${encodeURIComponent(metric)}&window_ms=${windowMs}`,
  );
  expect(response.ok(), `telemetry summary failed: ${response.url()}`).toBeTruthy();
  const payload = (await response.json()) as TelemetrySummaryResponse;
  const metrics = Array.isArray(payload.metrics) ? payload.metrics : [];
  if (metrics.length === 0) return metricRollupEmpty();
  const numericValues = (key: keyof MetricRollup) =>
    metrics
      .map((entry) => entry[key as keyof TelemetryMetricSummary])
      .filter((value): value is number => typeof value === "number" && Number.isFinite(value));
  const counts = numericValues("count");
  const sums = numericValues("sum");
  const mins = numericValues("min");
  const maxes = numericValues("max");
  const p50s = numericValues("p50");
  const p95s = numericValues("p95");
  const p99s = numericValues("p99");
  return {
    count: counts.reduce((total, value) => total + value, 0),
    sum: sums.reduce((total, value) => total + value, 0),
    min: mins.length > 0 ? Math.min(...mins) : null,
    max: maxes.length > 0 ? Math.max(...maxes) : null,
    p50: p50s.length > 0 ? Math.max(...p50s) : null,
    p95: p95s.length > 0 ? Math.max(...p95s) : null,
    p99: p99s.length > 0 ? Math.max(...p99s) : null,
  };
}

async function readTelemetryMetricEntries(
  request: APIRequestContext,
  metric: string,
  windowMs: number,
): Promise<TelemetryMetricSummary[]> {
  const response = await request.get(
    `/api/telemetry/summary?metric=${encodeURIComponent(metric)}&window_ms=${windowMs}`,
  );
  expect(response.ok(), `telemetry summary failed: ${response.url()}`).toBeTruthy();
  const payload = (await response.json()) as TelemetrySummaryResponse;
  return Array.isArray(payload.metrics) ? payload.metrics : [];
}

function sumMetricEntries(entries: readonly TelemetryMetricSummary[]): number {
  return entries.reduce(
    (total, entry) =>
      total + (typeof entry.sum === "number" && Number.isFinite(entry.sum) ? entry.sum : 0),
    0,
  );
}

function sumMetricEntriesByLabel(
  entries: readonly TelemetryMetricSummary[],
  labelName: string,
): Record<string, number> {
  const out: Record<string, number> = {};
  for (const entry of entries) {
    const labelValue = entry.labels?.[labelName] ?? "unknown";
    const value = typeof entry.sum === "number" && Number.isFinite(entry.sum) ? entry.sum : 0;
    out[labelValue] = (out[labelValue] ?? 0) + value;
  }
  return out;
}

async function readTelemetryMetrics(
  request: APIRequestContext,
  names: readonly string[],
  windowMs: number,
): Promise<Record<string, MetricRollup>> {
  const entries = await Promise.all(
    names.map(async (name) => [name, await readTelemetryMetric(request, name, windowMs)] as const),
  );
  return Object.fromEntries(entries);
}

async function calibrateClock(request: APIRequestContext): Promise<ClockCalibration> {
  const samples: ClockSample[] = [];
  for (let index = 0; index < CLOCK_SAMPLES; index += 1) {
    const startedAtMs = Date.now();
    const response = await request.get("/api/dev/clock");
    const endedAtMs = Date.now();
    expect(response.ok(), `clock calibration failed: ${response.url()}`).toBeTruthy();
    const payload = (await response.json()) as { daemon_unix_ms?: number };
    const daemonUnixMs =
      typeof payload.daemon_unix_ms === "number" && Number.isFinite(payload.daemon_unix_ms)
        ? payload.daemon_unix_ms
        : NaN;
    expect(Number.isFinite(daemonUnixMs), "clock calibration returned a daemon timestamp").toBeTruthy();
    const rttMs = endedAtMs - startedAtMs;
    const midpointMs = startedAtMs + rttMs / 2;
    samples.push({
      startedAtMs,
      endedAtMs,
      rttMs,
      daemonUnixMs,
      offsetMs: daemonUnixMs - midpointMs,
      uncertaintyMs: rttMs / 2,
    });
    await sleep(25);
  }
  const best = samples.slice().sort((left, right) => left.rttMs - right.rttMs)[0];
  if (!best) throw new Error("clock calibration produced no samples");
  return {
    samples,
    offsetMs: best.offsetMs,
    uncertaintyMs: best.uncertaintyMs,
    minRttMs: best.rttMs,
    p95RttMs: percentile(samples.map((sample) => sample.rttMs), 0.95),
  };
}

async function startVisibleProgressProbe(page: Page, faultMode: string): Promise<void> {
  await page.evaluate((mode) => {
    const win = window as RemoteDaemonLoadWindow;
    const samples: VisibleProgressSnapshot["samples"] = [];
    const startedAtMs = Date.now();
    let endedAtMs = startedAtMs;
    let lastSignature = "";
    let stopped = false;
    let observer: MutationObserver | null = null;
    let timer: number | null = null;

    const isVisible = (element: HTMLElement): boolean => {
      const style = window.getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return (
        style.display !== "none" &&
        style.visibility !== "hidden" &&
        Number(style.opacity || "1") > 0 &&
        rect.width > 0 &&
        rect.height > 0
      );
    };
    const activeTarget = (): HTMLElement | null => {
      const thread =
        document.querySelector<HTMLElement>(
          '.wb-session-slot[aria-hidden="false"] .wb-thread-scroller',
        ) ??
        document.querySelector<HTMLElement>(
          '.wb-session-slot[aria-hidden="false"] .wb-thread-live-tail',
        ) ??
        document.querySelector<HTMLElement>('.wb-session-slot[aria-hidden="false"]');
      return thread && isVisible(thread) ? thread : null;
    };
    const sample = () => {
      if (stopped) return;
      const target = activeTarget();
      if (!target) return;
      const text = target.innerText || target.textContent || "";
      const signature = `${text.length}:${text.slice(-320)}`;
      if (signature === lastSignature) return;
      lastSignature = signature;
      samples.push({
        at_ms: Date.now(),
        text_length: text.length,
        signature,
      });
    };
    if (mode === "pause-foreground-visible-progress") {
      const style = document.createElement("style");
      style.id = "ctx-remote-daemon-stream-load-fault";
      style.textContent = `
        .wb-session-slot[aria-hidden="false"] .wb-thread-scroller,
        .wb-session-slot[aria-hidden="false"] .wb-thread-live-tail {
          visibility: hidden !important;
        }
      `;
      document.head.appendChild(style);
    }
    observer = new MutationObserver(sample);
    observer.observe(document.body, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    timer = window.setInterval(sample, 250);
    sample();
    const snapshot = (): VisibleProgressSnapshot => ({
      startedAtMs,
      endedAtMs,
      samples: samples.slice(),
    });
    win.__ctxVisibleProgressProbe = {
      getSnapshot: snapshot,
      stop: () => {
        stopped = true;
        endedAtMs = Date.now();
        observer?.disconnect();
        observer = null;
        if (timer !== null) {
          window.clearInterval(timer);
          timer = null;
        }
        return snapshot();
      },
    };
  }, faultMode);
}

async function stopVisibleProgressProbe(page: Page): Promise<VisibleProgressSnapshot> {
  const snapshot = await page.evaluate(() => {
    const win = window as RemoteDaemonLoadWindow;
    return win.__ctxVisibleProgressProbe?.stop() ?? null;
  });
  if (!snapshot) throw new Error("visible progress probe was not installed");
  return snapshot;
}

async function readWorkspaceStreamTelemetrySamples(
  page: Page,
): Promise<WorkspaceStreamTelemetrySample[]> {
  return page.evaluate(() => {
    const win = window as RemoteDaemonLoadWindow;
    return win.__ctxWorkspaceStreamTelemetrySamples?.slice() ?? [];
  });
}

function summarizeVisibleCadence(snapshot: VisibleProgressSnapshot, activeUntilMs?: number | null): {
  sampleCount: number;
  maxVisibleSilenceMs: number;
  p95VisibleSilenceMs: number | null;
  gapsMs: number[];
} {
  const endedAtMs =
    typeof activeUntilMs === "number" && Number.isFinite(activeUntilMs)
      ? Math.max(snapshot.startedAtMs, Math.min(activeUntilMs, snapshot.endedAtMs))
      : snapshot.endedAtMs;
  const times = snapshot.samples
    .map((sample) => sample.at_ms)
    .filter((value) => Number.isFinite(value) && value <= endedAtMs)
    .sort((left, right) => left - right);
  const gaps: number[] = [];
  let cursor = snapshot.startedAtMs;
  for (const atMs of times) {
    if (atMs >= cursor) {
      gaps.push(atMs - cursor);
      cursor = atMs;
    }
  }
  gaps.push(Math.max(0, endedAtMs - cursor));
  return {
    sampleCount: times.length,
    maxVisibleSilenceMs: gaps.length > 0 ? Math.max(...gaps) : endedAtMs - snapshot.startedAtMs,
    p95VisibleSilenceMs: percentile(gaps, 0.95),
    gapsMs: gaps,
  };
}

function summarizeCorrectedReceiveLag(
  samples: readonly WorkspaceStreamTelemetrySample[],
  clockOffsetMs: number,
): {
  count: number;
  p50: number | null;
  p95: number | null;
  max: number | null;
} {
  const corrected = samples
    .filter((sample) => typeof sample.emittedAtMs === "number")
    .map((sample) => sample.receivedAtMs - Number(sample.emittedAtMs) + clockOffsetMs)
    .filter((value) => Number.isFinite(value) && value >= 0);
  return {
    count: corrected.length,
    p50: percentile(corrected, 0.5),
    p95: percentile(corrected, 0.95),
    max: corrected.length > 0 ? Math.max(...corrected) : null,
  };
}

async function stopStreamers(
  streamers: readonly BoundedWriter[],
): Promise<{ sent: number; failures: string[]; backpressure: number; stopErrors: string[] }> {
  const stopErrors: string[] = [];
  for (const streamer of streamers) {
    try {
      await streamer.stop();
    } catch (error) {
      stopErrors.push(formatUnknownError(error));
    }
  }
  const stats = streamers.map((streamer) => streamer.getStats());
  return {
    sent: stats.reduce((total, entry) => total + entry.sent, 0),
    failures: stats.flatMap((entry) => entry.failures),
    backpressure: stats.reduce((total, entry) => total + entry.backpressure, 0),
    stopErrors,
  };
}

test("workbench: remote daemon stream load keeps UI progress fresh", async ({
  page,
  request,
}, testInfo) => {
  test.skip(!ENABLED, "Set CTX_REMOTE_DAEMON_STREAM_SOAK=1 to run the remote daemon stream load proof.");
  test.setTimeout(480_000);

  const clock = await calibrateClock(request);

  const seed = await seedDummyWorkspace(request, {
    tasks: TASK_COUNT,
    sessionsPerTask: 1,
    turnsPerSession: TURNS_PER_SESSION,
    throttleMs: 1,
    messageBytes: MESSAGE_BYTES,
    messagePrefix: "remote stream fixture msg",
    includeToolSummaries: true,
    toolSummariesPerTurn: 3,
    seedTranscriptDirect: true,
  });

  const foregroundTaskId = seed.taskIds[0] ?? "";
  const foregroundSessionId = seed.sessionIdsByTask[foregroundTaskId]?.[0] ?? "";
  expect(foregroundSessionId).not.toBe("");
  const backgroundSessionIds = seed.taskIds
    .slice(1)
    .map((taskId) => seed.sessionIdsByTask[taskId]?.[0] ?? "")
    .filter((sessionId): sessionId is string => Boolean(sessionId));
  expect(backgroundSessionIds.length).toBeGreaterThan(0);

  let pageCrashed = false;
  page.on("crash", () => {
    pageCrashed = true;
  });

  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto(`/workspaces/${seed.workspaceId}?ctxE2E=1&loadtest=1`, {
    waitUntil: "domcontentloaded",
  });

  const rows = page.locator(".wb-task-row");
  const sessionView = page.locator('.wb-session-slot[aria-hidden="false"]');
  await expect(rows).toHaveCount(TASK_COUNT, { timeout: 30_000 });
  const focused = await page.evaluate(
    ({ taskId, sessionId }) => window.__ctxE2E?.focusTask?.(taskId, sessionId) ?? false,
    { taskId: foregroundTaskId, sessionId: foregroundSessionId },
  );
  expect(focused).toBe(true);
  await expect(sessionView).toContainText(/remote stream fixture msg 1\.1\./i, {
    timeout: 30_000,
  });
  await expect(page.locator(".wb-session-slot textarea.wb-active-textarea")).toBeVisible({
    timeout: 30_000,
  });
  await clearDiagnostics(page);

  await startVisibleProgressProbe(page, FAULT_MODE);

  const writerSessionIds = backgroundSessionIds.slice(0, Math.max(1, Math.min(STREAMERS, backgroundSessionIds.length)));
  const streamers = startBoundedBackgroundWriters(request, writerSessionIds);

  const probes: ProbeOutcome[] = [];
  const interruptRequestTimes: number[] = [];
  const requestListener = (req: Request) => {
    if (req.url().includes(`/api/sessions/${foregroundSessionId}/interrupt`)) {
      interruptRequestTimes.push(Date.now());
    }
  };
  page.on("request", requestListener);

  let streamerStats = {
    sent: 0,
    failures: [] as string[],
    backpressure: 0,
    stopErrors: [] as string[],
  };
  let interrupt = {
    marker: "",
    error: null as string | null,
    clickAtMs: null as number | null,
    requestAtMs: null as number | null,
    pendingAtMs: null as number | null,
    terminalAtMs: null as number | null,
    terminalStatus: null as string | null,
    clickToRequestMs: null as number | null,
    clickToPendingMs: null as number | null,
    clickToTerminalMs: null as number | null,
  };

  try {
    for (let index = 0; index < PROBE_COUNT; index += 1) {
      const marker = `remote-ui-progress-${index + 1}-${Date.now()}`;
      probes.push(await runForegroundProbe(page, request, foregroundSessionId, marker));
      await sleep(250);
    }

    const interruptMarker = `remote-ui-interrupt-${Date.now()}`;
    interrupt.marker = interruptMarker;
    try {
      await sendSessionMessage(request, foregroundSessionId, buildSlowPrompt(interruptMarker));
      const stopButton = page.getByRole("button", { name: "Stop" });
      await expect(stopButton).toBeVisible({ timeout: 20_000 });
      interrupt.clickAtMs = Date.now();
      await stopButton.click();
      await expect(page.getByRole("button", { name: "Stopping..." })).toBeVisible({
        timeout: MAX_CLICK_TO_PENDING_MS + 5000,
      });
      interrupt.pendingAtMs = Date.now();
      const terminal = await waitForTerminalInterrupted(
        request,
        foregroundSessionId,
        interruptMarker,
        MAX_CLICK_TO_TERMINAL_MS + 5000,
      );
      interrupt.terminalAtMs = terminal.terminalAtMs;
      interrupt.terminalStatus = terminal.status;
    } catch (error) {
      interrupt.error = formatUnknownError(error);
    }

    const streamDeadline = Date.now() + STREAM_TIMEOUT_MS;
    while (Date.now() < streamDeadline) {
      const streamEvents = await readTelemetryMetricEntries(
        request,
        "workbench.workspace_stream_event_count",
        180_000,
      );
      if (sumMetricEntries(streamEvents) >= MIN_STREAM_EVENTS) break;
      await sleep(500);
    }
  } finally {
    streamerStats = await stopStreamers(streamers);
    page.off("request", requestListener);
  }

  interrupt.requestAtMs = interruptRequestTimes[0] ?? null;
  if (interrupt.clickAtMs !== null && interrupt.requestAtMs !== null) {
    interrupt.clickToRequestMs = interrupt.requestAtMs - interrupt.clickAtMs;
  }
  if (interrupt.clickAtMs !== null && interrupt.pendingAtMs !== null) {
    interrupt.clickToPendingMs = interrupt.pendingAtMs - interrupt.clickAtMs;
  }
  if (interrupt.clickAtMs !== null && interrupt.terminalAtMs !== null) {
    interrupt.clickToTerminalMs = interrupt.terminalAtMs - interrupt.clickAtMs;
  }

  await sleep(1500);
  const visibleSnapshot = await stopVisibleProgressProbe(page);
  const streamTelemetrySamples = await readWorkspaceStreamTelemetrySamples(page);
  const visibleCadence = summarizeVisibleCadence(visibleSnapshot, interrupt.terminalAtMs);
  const backendToDomMs = probes
    .map((probe) => probe.backendToDomMs)
    .filter((value): value is number => typeof value === "number" && Number.isFinite(value));
  const correctedReceiveLag = summarizeCorrectedReceiveLag(
    streamTelemetrySamples,
    clock.offsetMs,
  );
  const correctedForegroundReceiveLag = summarizeCorrectedReceiveLag(
    streamTelemetrySamples.filter((sample) => sample.lane === "foreground"),
    clock.offsetMs,
  );
  const streamEventMetricEntries = await readTelemetryMetricEntries(
    request,
    "workbench.workspace_stream_event_count",
    180_000,
  );
  const streamEventCountsByType = sumMetricEntriesByLabel(streamEventMetricEntries, "event_type");
  const streamEventCountsByLane = sumMetricEntriesByLabel(streamEventMetricEntries, "lane");
  const telemetryMetrics = await readTelemetryMetrics(
    request,
    [
      "workbench.workspace_stream_event_count",
      "workbench.client_receive_lag_ms",
      "workbench.session_replica_apply_lag_ms",
      "workbench.session_replica_apply_duration_ms",
      "workbench.final_ws_to_dom_ms",
      "workbench.final_ingress_to_dom_ms",
      "workbench.foreground_queue_age_ms",
      "workbench.workspace_backlog_age_ms",
      "workbench.interrupt_click_to_pending_ms",
      "workbench.foreground_gap_recovery_timeout_count",
      "workbench.workspace_stream_reset_count",
      "workbench.late_chunk_after_terminal_count",
      "workbench.projection_or_seq_regression_count",
      "workbench.gap_repair_mismatch_count",
      "workbench.switch_stale_visible_count",
      "workbench.nav_thread_activity_mismatch_count",
    ],
    180_000,
  );
  const diagnostics = await getDiagnostics(page);
  const browserLoadTest = await page.evaluate(() => {
    const win = window as Window & {
      __ctxLoadTestTelemetry?: {
        getSummary?: () => unknown;
      };
    };
    return win.__ctxLoadTestTelemetry?.getSummary?.() ?? null;
  });

  const summary = {
    run: {
      remoteMode: REMOTE_MODE,
      faultMode: FAULT_MODE || null,
      baseUrl: process.env.CTX_E2E_BASE_URL ?? null,
      workspaceId: seed.workspaceId,
      foregroundTaskId,
      foregroundSessionId,
      backgroundSessionCount: backgroundSessionIds.length,
      pageCrashed,
    },
    budgets: {
      minStreamEvents: MIN_STREAM_EVENTS,
      minSessionHeadDeltas: MIN_SESSION_HEAD_DELTAS,
      maxClockUncertaintyMs: MAX_CLOCK_UNCERTAINTY_MS,
      maxVisibleSilenceMs: MAX_VISIBLE_SILENCE_MS,
      maxBackendToDomP95Ms: MAX_BACKEND_TO_DOM_P95_MS,
      maxBackendToDomMs: MAX_BACKEND_TO_DOM_MS,
      maxClientReceiveLagMs: MAX_CLIENT_RECEIVE_LAG_MS,
      maxReplicaApplyLagMs: MAX_REPLICA_APPLY_LAG_MS,
      maxClickToPendingMs: MAX_CLICK_TO_PENDING_MS,
      maxClickToTerminalMs: MAX_CLICK_TO_TERMINAL_MS,
    },
    clock,
    load: {
      streamers: STREAMERS,
      intervalMs: STREAM_INTERVAL_MS,
      messageBytes: MESSAGE_BYTES,
      backgroundMessagesSent: streamerStats.sent,
      streamerFailures: streamerStats.failures,
      streamerBackpressure: streamerStats.backpressure,
      streamerStopErrors: streamerStats.stopErrors,
      streamEventCount: sumMetricEntries(streamEventMetricEntries),
      streamEventCountsByType,
      streamEventCountsByLane,
      streamTelemetrySampleCount: streamTelemetrySamples.length,
    },
    visibleProgress: {
      cadence: visibleCadence,
      samplePreview: visibleSnapshot.samples.slice(-12),
    },
    probes: {
      outcomes: probes,
      backendToDomMs,
      p50BackendToDomMs: percentile(backendToDomMs, 0.5),
      p95BackendToDomMs: percentile(backendToDomMs, 0.95),
      maxBackendToDomMs: backendToDomMs.length > 0 ? Math.max(...backendToDomMs) : null,
    },
    receiveLag: {
      correctedAll: correctedReceiveLag,
      correctedForeground: correctedForegroundReceiveLag,
      telemetry: telemetryMetrics["workbench.client_receive_lag_ms"] ?? metricRollupEmpty(),
    },
    interrupt,
    telemetryMetrics,
    diagnostics: diagnostics.map((entry) => ({
      source: entry.source,
      code: entry.code,
      severity: entry.severity,
      message: entry.message,
    })),
    browserLoadTest,
  };

  await testInfo.attach("remote-daemon-stream-load-metrics.json", {
    body: JSON.stringify(summary, null, 2),
    contentType: "application/json",
  });
  await fs.writeFile(
    testInfo.outputPath("remote-daemon-stream-load-metrics.json"),
    JSON.stringify(summary, null, 2),
    "utf8",
  );
  console.log(`remote daemon stream load summary: ${JSON.stringify(summary)}`);

  expect(clock.uncertaintyMs).toBeLessThanOrEqual(MAX_CLOCK_UNCERTAINTY_MS);
  expect(sumMetricEntries(streamEventMetricEntries)).toBeGreaterThanOrEqual(MIN_STREAM_EVENTS);
  expect(streamEventCountsByType.session_head_delta ?? 0).toBeGreaterThanOrEqual(MIN_SESSION_HEAD_DELTAS);
  expect(streamEventCountsByLane.foreground ?? 0).toBeGreaterThan(0);
  expect(streamerStats.failures).toEqual([]);
  expect(streamerStats.stopErrors).toEqual([]);
  expect(pageCrashed).toBe(false);
  expect(visibleCadence.maxVisibleSilenceMs).toBeLessThanOrEqual(MAX_VISIBLE_SILENCE_MS);
  expect(visibleCadence.sampleCount).toBeGreaterThan(0);
  expect(probes.every((probe) => !probe.timedOut)).toBe(true);
  expect(backendToDomMs.length).toBe(PROBE_COUNT);
  expect(percentile(backendToDomMs, 0.95) ?? Infinity).toBeLessThanOrEqual(
    MAX_BACKEND_TO_DOM_P95_MS,
  );
  expect(backendToDomMs.length > 0 ? Math.max(...backendToDomMs) : Infinity).toBeLessThanOrEqual(
    MAX_BACKEND_TO_DOM_MS,
  );
  expect(telemetryMetrics["workbench.client_receive_lag_ms"]?.count ?? 0).toBeGreaterThan(0);
  expect(correctedReceiveLag.count).toBeGreaterThan(0);
  expect(correctedReceiveLag.p95 ?? Infinity).toBeLessThanOrEqual(MAX_CLIENT_RECEIVE_LAG_MS);
  expect(telemetryMetrics["workbench.session_replica_apply_lag_ms"]?.count ?? 0).toBeGreaterThan(0);
  expect(telemetryMetrics["workbench.session_replica_apply_lag_ms"]?.p95 ?? Infinity).toBeLessThanOrEqual(
    MAX_REPLICA_APPLY_LAG_MS,
  );
  expect(telemetryMetrics["workbench.session_replica_apply_duration_ms"]?.count ?? 0).toBeGreaterThan(0);
  expect(interrupt.error).toBeNull();
  expect(interrupt.clickToRequestMs ?? Infinity).toBeLessThanOrEqual(MAX_CLICK_TO_PENDING_MS);
  expect(interrupt.clickToPendingMs ?? Infinity).toBeLessThanOrEqual(MAX_CLICK_TO_PENDING_MS);
  expect(interrupt.clickToTerminalMs ?? Infinity).toBeLessThanOrEqual(MAX_CLICK_TO_TERMINAL_MS);
  expect(
    ["interrupted", "cancelled", "canceled"].includes(String(interrupt.terminalStatus ?? "")),
  ).toBe(true);
  expect(telemetryMetrics["workbench.foreground_gap_recovery_timeout_count"]?.count ?? 0).toBe(0);
  expect(telemetryMetrics["workbench.late_chunk_after_terminal_count"]?.count ?? 0).toBe(0);
  expect(telemetryMetrics["workbench.gap_repair_mismatch_count"]?.count ?? 0).toBe(0);
  expect(telemetryMetrics["workbench.switch_stale_visible_count"]?.count ?? 0).toBe(0);
});
