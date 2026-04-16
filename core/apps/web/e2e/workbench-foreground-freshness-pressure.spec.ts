import fs from "fs/promises";
import type { APIRequestContext } from "playwright/test";
import { test, expect } from "./fixtures";
import { clearDiagnostics, getDiagnostics } from "./utils/diagnostics";
import { seedDummyWorkspace, startStreamingMessages } from "./utils/seedDummyWorkspace";

const PROBE_ENABLED = process.env.CTX_FOREGROUND_FRESHNESS_PROBE === "1";
const GUARDRAIL_ENABLED = process.env.CTX_FOREGROUND_FRESHNESS_GUARDRAIL === "1";
const TASK_COUNT = Number(process.env.CTX_FOREGROUND_FRESHNESS_TASKS ?? "10");
const TURNS_PER_SESSION = Number(process.env.CTX_FOREGROUND_FRESHNESS_TURNS ?? "2");
const MESSAGE_BYTES = Number(process.env.CTX_FOREGROUND_FRESHNESS_MESSAGE_BYTES ?? "2200");
const BACKGROUND_WAVES = Number(process.env.CTX_FOREGROUND_FRESHNESS_BACKGROUND_WAVES ?? "2");
const BACKGROUND_WAVE_DELAY_MS = Number(
  process.env.CTX_FOREGROUND_FRESHNESS_BACKGROUND_WAVE_DELAY_MS ?? "1200",
);
const BACKGROUND_STREAM_INTERVAL_MS = Number(
  process.env.CTX_FOREGROUND_FRESHNESS_BACKGROUND_STREAM_INTERVAL_MS ?? "0",
);
const BACKGROUND_STREAM_DURATION_MS = Number(
  process.env.CTX_FOREGROUND_FRESHNESS_BACKGROUND_STREAM_DURATION_MS ?? "0",
);
const PROMPT_BODY_LINES = Number(process.env.CTX_FOREGROUND_FRESHNESS_PROMPT_BODY_LINES ?? "90");
const PROBE_RUNS = Number(process.env.CTX_FOREGROUND_FRESHNESS_PROBE_RUNS ?? "3");
const HEAD_POLL_MS = Number(process.env.CTX_FOREGROUND_FRESHNESS_HEAD_POLL_MS ?? "50");
const MAX_BACKEND_TO_DOM_MS = Number(process.env.CTX_FOREGROUND_FRESHNESS_MAX_BACKEND_TO_DOM_MS ?? "0");

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

type SlowPromptTool = {
  kind: string;
  title: string;
  input: Record<string, string>;
  output_text: string;
};

const slowPromptBody = Array.from(
  { length: PROMPT_BODY_LINES },
  (_, index) => `foreground freshness body line ${index + 1}`,
).join("\n");

function percentile(values: number[], p: number): number | null {
  if (values.length === 0) return null;
  const sorted = values.slice().sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * p) - 1));
  return Math.round(sorted[index]! * 10) / 10;
}

const buildSlowPrompt = (marker: string, label: string): string => {
  const tools: SlowPromptTool[] = Array.from({ length: 4 }, (_, index) => ({
    kind: "execute",
    title: `${label} tool ${index + 1}`,
    input: { command: `printf '${label}-${index + 1}'` },
    output_text: `${label} output ${index + 1}`,
  }));
  return `slow-diff-test stream-assistant-partials emit-thought ${marker}
${slowPromptBody}
[[tool_calls]]
${JSON.stringify(tools)}
[[/tool_calls]]`;
};

async function pageWait(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function sendSessionMessage(
  request: APIRequestContext,
  sessionId: string,
  content: string,
): Promise<void> {
  const response = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content,
      delivery: "immediate",
    },
  });
  expect(response.ok(), `message send failed: ${response.url()}`).toBeTruthy();
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
        typeof message?.content === "string" &&
        message.content.includes(marker),
    );
    if (userMessage?.id) {
      const turn = turns.find((entry) => entry?.user_message_id === userMessage.id);
      const assistantMessage = messages.find(
        (message) =>
          message?.turn_id === turn?.turn_id &&
          message?.role === "assistant" &&
          typeof message?.content === "string" &&
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
    await pageWait(HEAD_POLL_MS);
  }
  throw new Error(`foreground turn did not complete for marker ${marker}`);
}

async function runBackgroundPressure(
  request: APIRequestContext,
  sessionIds: string[],
  runNumber: number,
): Promise<void> {
  for (let wave = 0; wave < BACKGROUND_WAVES; wave += 1) {
    await Promise.all(
      sessionIds.map((sessionId, index) =>
        sendSessionMessage(
          request,
          sessionId,
          buildSlowPrompt(
            `background-pressure-${runNumber}-${wave + 1}-${index + 1}`,
            `bg-${runNumber}-${wave + 1}-${index + 1}`,
          ),
        ),
      ),
    );
    if (wave < BACKGROUND_WAVES - 1) {
      await pageWait(BACKGROUND_WAVE_DELAY_MS);
    }
  }
}

test.use({ browserName: "chromium" });

test("workbench: foreground final freshness stays tight under background pressure", async ({
  page,
  request,
}, testInfo) => {
  test.skip(
    !PROBE_ENABLED && !GUARDRAIL_ENABLED,
    "Set CTX_FOREGROUND_FRESHNESS_PROBE=1 or CTX_FOREGROUND_FRESHNESS_GUARDRAIL=1 to run the foreground freshness pressure probe.",
  );
  test.setTimeout(300_000);

  const seed = await seedDummyWorkspace(request, {
    tasks: TASK_COUNT,
    sessionsPerTask: 1,
    turnsPerSession: TURNS_PER_SESSION,
    throttleMs: 1,
    messageBytes: MESSAGE_BYTES,
    messagePrefix: "freshness fixture msg",
  });

  const foregroundTaskId = seed.taskIds[0] ?? "";
  const foregroundSessionId = seed.sessionIdsByTask[foregroundTaskId]?.[0] ?? "";
  expect(foregroundSessionId).not.toBe("");
  const backgroundSessionIds = seed.taskIds
    .slice(1)
    .map((taskId) => seed.sessionIdsByTask[taskId]?.[0] ?? "")
    .filter(Boolean);
  expect(backgroundSessionIds.length).toBeGreaterThan(0);

  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto(`/workspaces/${seed.workspaceId}?ctxE2E=1&loadtest=1`, {
    waitUntil: "domcontentloaded",
  });

  const rows = page.locator(".wb-task-row");
  const sessionView = page.locator('.wb-session-slot[aria-hidden="false"]');
  await expect(rows).toHaveCount(TASK_COUNT, { timeout: 30_000 });

  const openForegroundTask = async () => {
    const focused = await page.evaluate(
      ({ taskId, sessionId }) => window.__ctxE2E?.focusTask?.(taskId, sessionId) ?? false,
      { taskId: foregroundTaskId, sessionId: foregroundSessionId },
    );
    expect(focused).toBe(true);
    await expect(sessionView).toContainText(/freshness fixture msg 1\.1\./i, { timeout: 30_000 });
    await expect(page.locator(".wb-session-slot textarea.wb-active-textarea")).toBeVisible({
      timeout: 30_000,
    });
  };

  const runs: Array<{
    run: number;
    backendToDomMs: number;
    marker: string;
    turnId: string;
    diagnosticCodes: string[];
  }> = [];

  for (let run = 0; run < PROBE_RUNS; run += 1) {
    await openForegroundTask();
    await clearDiagnostics(page);
    const backgroundPressure = runBackgroundPressure(request, backgroundSessionIds, run + 1);
    const streamer =
      BACKGROUND_STREAM_INTERVAL_MS > 0 && BACKGROUND_STREAM_DURATION_MS > 0
        ? startStreamingMessages(request, {
          sessionIds: backgroundSessionIds,
          intervalMs: BACKGROUND_STREAM_INTERVAL_MS,
          durationMs: BACKGROUND_STREAM_DURATION_MS,
          messageBytes: MESSAGE_BYTES,
          includeToolSummaries: true,
          toolSummariesPerTurn: 4,
          messagePrefix: `background-stream-${run + 1}`,
        })
        : null;
    await pageWait(200);

    try {
      const marker = `foreground-freshness-${run + 1}-${Date.now()}`;
      await sendSessionMessage(
        request,
        foregroundSessionId,
        buildSlowPrompt(marker, `foreground-${run + 1}`),
      );

      const { backendReadyAtMs, turnId } = await waitForForegroundTurnCompletion(
        request,
        foregroundSessionId,
        marker,
        45_000,
      );
      const domVisibleAtMs = await page.waitForFunction(
        ({ text }) => {
          const session = document.querySelector('.wb-session-slot[aria-hidden="false"]');
          return session?.textContent?.includes(text) ? Date.now() : null;
        },
        { text: marker },
        { timeout: 45_000 },
      );
      const domVisibleAt = await domVisibleAtMs.jsonValue();
      expect(typeof domVisibleAt).toBe("number");
      const diagnostics = await getDiagnostics(page);
      runs.push({
        run: run + 1,
        backendToDomMs: Number(domVisibleAt) - backendReadyAtMs,
        marker,
        turnId,
        diagnosticCodes: diagnostics.map((entry) => `${entry.source}:${entry.code}:${entry.severity}`),
      });
      await backgroundPressure;
    } finally {
      if (streamer) {
        await streamer.stop();
      }
    }
  }

  const backendToDomMs = runs.map((entry) => entry.backendToDomMs);
  const summary = {
    workspaceId: seed.workspaceId,
    taskCount: TASK_COUNT,
    turnsPerSession: TURNS_PER_SESSION,
    backgroundWaves: BACKGROUND_WAVES,
    backgroundWaveDelayMs: BACKGROUND_WAVE_DELAY_MS,
    backgroundStreamIntervalMs: BACKGROUND_STREAM_INTERVAL_MS,
    backgroundStreamDurationMs: BACKGROUND_STREAM_DURATION_MS,
    probeRuns: runs,
    backendToDomMs,
    p50Ms: percentile(backendToDomMs, 0.5),
    p95Ms: percentile(backendToDomMs, 0.95),
    maxMs: backendToDomMs.length > 0 ? Math.max(...backendToDomMs) : null,
  };

  await testInfo.attach("foreground-freshness-pressure.json", {
    body: JSON.stringify(summary, null, 2),
    contentType: "application/json",
  });
  await fs.writeFile(
    testInfo.outputPath("foreground-freshness-pressure.json"),
    JSON.stringify(summary, null, 2),
    "utf8",
  );

  console.log(`foreground freshness summary: ${JSON.stringify(summary)}`);

  if (GUARDRAIL_ENABLED || MAX_BACKEND_TO_DOM_MS > 0) {
    const thresholdMs = GUARDRAIL_ENABLED ? 150 : MAX_BACKEND_TO_DOM_MS;
    expect(summary.p95Ms ?? Infinity).toBeLessThanOrEqual(thresholdMs);
  }
});
