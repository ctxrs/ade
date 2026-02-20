import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import type { APIRequestContext, Page } from "playwright/test";
import {
  configureHarnessEndpointAuthViaModal,
  selectHarnessForComposer,
} from "./utils/harnessEndpointAuth";
import {
  OPENROUTER_ENDPOINT_FIRST_PASS_HARNESSES,
} from "./utils/harnessEndpointMatrix";

type Outcome = "pass" | "skip" | "fail";

type HarnessRunRecord = {
  provider_id: string;
  menu_label: string;
  provider_installed: boolean;
  provider_health: string;
  provider_diagnostics: string[];
  auth_saved: boolean;
  auth_detail: string;
  harness_selected: boolean;
  harness_detail: string;
  session_started: boolean;
  session_id: string | null;
  model_id: string | null;
  terminal_status: string | null;
  assistant_messages: number;
  result: Outcome;
  reason: string;
  elapsed_ms: number;
};

type ProviderHealth = {
  installed: boolean;
  health: string;
  diagnostics: string[];
};

type ActiveTaskSummary = {
  taskId: string;
  title: string;
  primarySessionId: string;
  latestSessionId: string;
};

type RunContext = {
  taskId: string;
  sessionId: string;
};

type TerminalState = {
  done: boolean;
  terminalStatus: string | null;
  assistantMessages: number;
  errorMessage: string | null;
  modelId: string | null;
};

type ProviderVerifyResult = {
  ok: boolean;
  status: string;
  detail: string;
};

const DEFAULT_OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_E2E_AUTH_TOKEN = "ctx-e2e-auth-token";
const TERMINAL_TURN_STATUSES = new Set(["completed", "failed", "interrupted"]);

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

const asArray = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

const readString = (value: unknown): string => (typeof value === "string" ? value : "");

const firstText = (...values: unknown[]): string => {
  for (const value of values) {
    const text = readString(value).trim();
    if (text) return text;
  }
  return "";
};

const normalizeErrorMessage = (raw: string): string => raw.replace(/\s+/g, " ").trim();

const isLikelyRuntimeSkip = (message: string): boolean => {
  const normalized = message.toLowerCase();
  return [
    "command not found",
    "no such file or directory",
    "not installed",
    "missing",
    "enoent",
    "failed to spawn",
    "unavailable",
    "not healthy",
    "install",
  ].some((token) => normalized.includes(token));
};

async function providerHealthMap(request: APIRequestContext): Promise<Record<string, ProviderHealth>> {
  const resp = await request.get("/api/providers");
  if (!resp.ok()) {
    throw new Error(`failed to read providers (${resp.status()})`);
  }
  const rows = asArray(await resp.json());
  const out: Record<string, ProviderHealth> = {};
  for (const row of rows) {
    const rec = asRecord(row);
    const providerId = readString(rec.provider_id);
    if (!providerId) continue;
    const diagnostics = asArray(rec.diagnostics).map((entry) => readString(entry)).filter(Boolean);
    out[providerId] = {
      installed: rec.installed === true,
      health: readString(rec.health) || "unknown",
      diagnostics,
    };
  }
  return out;
}

async function readActiveTaskSummaries(
  request: APIRequestContext,
  workspaceId: string,
): Promise<ActiveTaskSummary[]> {
  const resp = await request.get(`/api/workspaces/${workspaceId}/active_snapshot`);
  if (!resp.ok()) return [];
  const data = asRecord(await resp.json());
  const active = asRecord(data.active);
  return asArray(active.tasks)
    .map((entry): ActiveTaskSummary | null => {
      const summary = asRecord(entry);
      const task = asRecord(summary.task);
      const taskId = readString(task.id);
      if (!taskId) return null;
      const title = readString(task.title);
      const primarySessionId = readString(task.primary_session_id);
      const sessions = asArray(summary.sessions).map((sessionEntry) => asRecord(sessionEntry));
      const latestSession = sessions.length > 0 ? asRecord(sessions[0].session) : {};
      const latestSessionId = readString(latestSession.id);
      return {
        taskId,
        title,
        primarySessionId,
        latestSessionId,
      };
    })
    .filter((entry): entry is ActiveTaskSummary => Boolean(entry));
}

async function waitForRunContext(opts: {
  request: APIRequestContext;
  workspaceId: string;
  promptMarker: string;
  beforeTaskIds: Set<string>;
  timeoutMs?: number;
}): Promise<RunContext> {
  const { request, workspaceId, promptMarker, beforeTaskIds, timeoutMs = 30_000 } = opts;
  let resolved: RunContext | null = null;

  await expect
    .poll(
      async () => {
        const tasks = await readActiveTaskSummaries(request, workspaceId);
        const byNewTask = tasks.find((task) => !beforeTaskIds.has(task.taskId));
        const byPromptMarker = tasks.find((task) => task.title.includes(promptMarker));
        const selected = byNewTask ?? byPromptMarker;
        if (!selected) return "";
        const sessionId = firstText(selected.latestSessionId, selected.primarySessionId);
        if (!sessionId) return "";
        resolved = { taskId: selected.taskId, sessionId };
        return sessionId;
      },
      { timeout: timeoutMs, intervals: [500, 1_000, 2_000] },
    )
    .not.toBe("");

  if (!resolved) throw new Error("session id did not resolve after start request");
  return resolved;
}

function extractErrorMessage(snapshot: Record<string, unknown>): string {
  const summary = asRecord(snapshot.summary);
  const head = asRecord(snapshot.head);
  const turns = asArray(head.turns).map((entry) => asRecord(entry));
  const events = asArray(head.events).map((entry) => asRecord(entry));

  for (let i = events.length - 1; i >= 0; i -= 1) {
    const event = events[i];
    const payload = asRecord(event.payload_json);
    const message = firstText(
      payload.message,
      payload.error,
      payload.reason,
      payload.detail,
      payload.stderr,
      payload.stdout,
    );
    if (message) {
      const eventType = readString(event.event_type);
      return normalizeErrorMessage(eventType ? `[${eventType}] ${message}` : message);
    }
  }

  const lastTurn = turns.length > 0 ? turns[turns.length - 1] : {};
  return normalizeErrorMessage(
    firstText(
      asRecord(summary.session).status,
      asRecord(head.session).status,
      lastTurn.status,
      asRecord(summary.activity).last_turn_status,
      asRecord(head.activity).last_turn_status,
    ) || "no explicit error payload",
  );
}

function toTerminalState(snapshot: Record<string, unknown>): TerminalState {
  const summary = asRecord(snapshot.summary);
  const head = asRecord(snapshot.head);
  const summaryActivity = asRecord(summary.activity);
  const headActivity = asRecord(head.activity);
  const turns = asArray(head.turns).map((entry) => asRecord(entry));
  const messages = asArray(head.messages).map((entry) => asRecord(entry));

  const lastTurn = turns.length > 0 ? turns[turns.length - 1] : {};
  const terminalStatus = firstText(
    lastTurn.status,
    headActivity.last_turn_status,
    summaryActivity.last_turn_status,
  ).toLowerCase() || null;

  const assistantMessages = messages.filter((message) => {
    if (readString(message.role) !== "assistant") return false;
    return readString(message.content).trim().length > 0;
  }).length;

  const summaryWorking = summaryActivity.is_working === true;
  const headWorking = headActivity.is_working === true;
  const done = terminalStatus
    ? TERMINAL_TURN_STATUSES.has(terminalStatus)
    : !summaryWorking && !headWorking && assistantMessages > 0;

  const modelId = firstText(asRecord(summary.session).model_id, asRecord(head.session).model_id) || null;
  const failedOrInterrupted = terminalStatus === "failed" || terminalStatus === "interrupted";

  return {
    done,
    terminalStatus,
    assistantMessages,
    errorMessage: failedOrInterrupted || !done ? extractErrorMessage(snapshot) : null,
    modelId,
  };
}

async function waitForTerminalState(opts: {
  request: APIRequestContext;
  sessionId: string;
  timeoutMs?: number;
}): Promise<TerminalState> {
  const { request, sessionId, timeoutMs = 120_000 } = opts;
  let resolved: TerminalState | null = null;

  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/sessions/${sessionId}/snapshot?include_events=1&limit=80`);
        if (!resp.ok()) return "";
        const state = toTerminalState(asRecord(await resp.json()));
        if (!state.done) return "";
        resolved = state;
        return "done";
      },
      { timeout: timeoutMs, intervals: [1_000, 2_000, 3_000] },
    )
    .toBe("done");

  if (!resolved) {
    throw new Error(`session ${sessionId} did not reach terminal state`);
  }

  return resolved;
}

async function ensureNewTaskComposerVisible(page: Page): Promise<void> {
  const composer = page.locator("textarea.wb-composer-textarea").first();
  if (await composer.isVisible().catch(() => false)) return;

  const newTaskButton = page.getByRole("button", { name: "New task" }).first();
  await expect(newTaskButton).toBeVisible({ timeout: 15_000 });
  await newTaskButton.click();
  await expect(composer).toBeVisible({ timeout: 20_000 });
}

async function verifyProviderForWorkspace(opts: {
  request: APIRequestContext;
  workspaceId: string;
  providerId: string;
}): Promise<ProviderVerifyResult> {
  const { request, workspaceId, providerId } = opts;
  const resp = await request.post(`/api/workspaces/${workspaceId}/providers/${providerId}/verify`, {
    data: {},
  });
  if (!resp.ok()) {
    const body = asRecord(await resp.json().catch(() => ({})));
    const message = firstText(body.error, body.message, `verify request failed (${resp.status()})`);
    return {
      ok: false,
      status: "error",
      detail: normalizeErrorMessage(message),
    };
  }

  const body = asRecord(await resp.json());
  const status = firstText(body.status, "unknown").toLowerCase();
  const message = normalizeErrorMessage(firstText(body.message, ""));
  return {
    ok: status === "ok",
    status,
    detail: message || `status=${status}`,
  };
}

function formatResultTable(results: HarnessRunRecord[]): string {
  const header = "provider | auth | session | result | reason";
  const rows = results.map((row) => {
    const auth = row.auth_saved ? "ok" : "no";
    const session = row.session_started ? (row.terminal_status ?? "started") : "none";
    return `${row.provider_id} | ${auth} | ${session} | ${row.result} | ${row.reason}`;
  });
  return [header, ...rows].join("\n");
}

test("workbench: endpoint harness OpenRouter matrix first pass", async ({ page, request }, testInfo) => {
  test.setTimeout(25 * 60_000);

  if ((process.env.CTX_E2E_TIER ?? "") !== "endpoint-ui") {
    test.skip(true, "set CTX_E2E_TIER=endpoint-ui to run endpoint harness matrix test");
  }

  const apiKey = (process.env.OPENROUTER_API_KEY ?? "").trim();
  if (!apiKey) {
    test.skip(true, "missing OPENROUTER_API_KEY");
  }

  const baseUrl = (process.env.OPENROUTER_BASE_URL ?? "").trim() || DEFAULT_OPENROUTER_BASE_URL;
  const authToken = (process.env.CTX_E2E_AUTH_TOKEN ?? "").trim() || DEFAULT_E2E_AUTH_TOKEN;

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "openrouter endpoint harness matrix e2e\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `or-endpoint-matrix-${Date.now()}`;
  const createWorkspaceResp = await request.post("/api/workspaces", {
    data: {
      root_path: repo,
      name: workspaceName,
    },
  });
  expect(createWorkspaceResp.ok()).toBeTruthy();
  const createdWorkspace = asRecord(await createWorkspaceResp.json());
  const workspaceId = readString(createdWorkspace.id);
  expect(workspaceId).not.toBe("");

  await page.goto(
    `/workspaces/${workspaceId}?token=${encodeURIComponent(authToken)}&desktop_ui=1`,
  );
  try {
    await expect(page).toHaveURL(new RegExp(`/workspaces/${workspaceId}`), { timeout: 20_000 });
    await expect(page.locator(".wb-main")).toBeVisible({ timeout: 20_000 });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`failed to open workbench workspace ${workspaceId}: ${message}`);
  }

  const providers = await providerHealthMap(request);
  const results: HarnessRunRecord[] = [];

  for (const entry of OPENROUTER_ENDPOINT_FIRST_PASS_HARNESSES) {
    const startMs = Date.now();
    const provider = providers[entry.providerId] ?? {
      installed: false,
      health: "unknown",
      diagnostics: ["provider not listed by /api/providers"],
    };

    const baseRecord: HarnessRunRecord = {
      provider_id: entry.providerId,
      menu_label: entry.menuLabel,
      provider_installed: provider.installed,
      provider_health: provider.health,
      provider_diagnostics: provider.diagnostics,
      auth_saved: false,
      auth_detail: "",
      harness_selected: false,
      harness_detail: "",
      session_started: false,
      session_id: null,
      model_id: null,
      terminal_status: null,
      assistant_messages: 0,
      result: "fail",
      reason: "",
      elapsed_ms: 0,
    };

    try {
      await ensureNewTaskComposerVisible(page);

      if (!provider.installed || provider.health !== "ok") {
        const reason = firstText(provider.diagnostics[0], `provider health=${provider.health}`);
        results.push({
          ...baseRecord,
          result: "skip",
          reason: reason || "provider unavailable",
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const authResult = await configureHarnessEndpointAuthViaModal(page, entry, apiKey, baseUrl);
      const authLikelyAlreadyConfigured =
        !authResult.ok && authResult.detail.toLowerCase().includes("already be configured");
      const authSaved = authResult.ok || authLikelyAlreadyConfigured;
      if (!authSaved) {
        results.push({
          ...baseRecord,
          auth_saved: false,
          auth_detail: authResult.detail,
          result: "fail",
          reason: `auth save failed: ${authResult.detail}`,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const harnessSelect = await selectHarnessForComposer(page, entry);
      if (!harnessSelect.ok) {
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: false,
          harness_detail: harnessSelect.detail,
          result: "fail",
          reason: `harness select failed: ${harnessSelect.detail}`,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const verify = await verifyProviderForWorkspace({
        request,
        workspaceId,
        providerId: entry.providerId,
      });
      if (!verify.ok) {
        const reason = `verify failed: ${verify.detail}`;
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          result: isLikelyRuntimeSkip(verify.detail) ? "skip" : "fail",
          reason,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const promptMarker = `or-matrix-${entry.providerId}-${Date.now()}`;
      const prompt = `${promptMarker}: reply with exactly the word pong`;
      const beforeTaskIds = new Set((await readActiveTaskSummaries(request, workspaceId)).map((task) => task.taskId));

      const newComposer = page.locator("textarea.wb-composer-textarea").first();
      await expect(newComposer).toBeVisible({ timeout: 20_000 });
      await newComposer.fill(prompt);
      const sendButton = page.getByRole("button", { name: "Send" });
      await expect(sendButton).toBeEnabled({ timeout: 10_000 });
      await sendButton.click();

      let runContext: RunContext | null = null;
      try {
        runContext = await waitForRunContext({
          request,
          workspaceId,
          promptMarker,
          beforeTaskIds,
        });
      } catch {
        const banner = normalizeErrorMessage(
          firstText(await page.locator(".wb-banner").first().textContent().catch(() => "")),
        );
        const reason = banner || "session did not start";
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          result: isLikelyRuntimeSkip(reason) ? "skip" : "fail",
          reason,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const terminal = await waitForTerminalState({
        request,
        sessionId: runContext.sessionId,
      });

      let result: Outcome = "fail";
      let reason = "session ended without assistant completion";
      if (terminal.terminalStatus === "completed" && terminal.assistantMessages > 0) {
        result = "pass";
        reason = "assistant completion observed";
      } else {
        const errorMessage = terminal.errorMessage || "session ended without explicit error payload";
        if (isLikelyRuntimeSkip(errorMessage)) {
          result = "skip";
          reason = errorMessage;
        } else {
          result = "fail";
          reason = errorMessage;
        }
      }

      results.push({
        ...baseRecord,
        auth_saved: true,
        auth_detail: authResult.detail,
        harness_selected: true,
        harness_detail: harnessSelect.detail,
        session_started: true,
        session_id: runContext.sessionId,
        model_id: terminal.modelId,
        terminal_status: terminal.terminalStatus,
        assistant_messages: terminal.assistantMessages,
        result,
        reason,
        elapsed_ms: Date.now() - startMs,
      });
    } catch (error) {
      const message = normalizeErrorMessage(error instanceof Error ? error.message : String(error));
      results.push({
        ...baseRecord,
        result: isLikelyRuntimeSkip(message) ? "skip" : "fail",
        reason: message,
        elapsed_ms: Date.now() - startMs,
      });
    }
  }

  await testInfo.attach("openrouter-endpoint-matrix-results", {
    body: JSON.stringify(
      {
        generated_at: new Date().toISOString(),
        workspace_id: workspaceId,
        suite: "openrouter-endpoint-first-pass",
        base_url: baseUrl,
        model_preference: "session default (no CTX_TOKENS_MODEL override)",
        results,
      },
      null,
      2,
    ),
    contentType: "application/json",
  });

  const summary = formatResultTable(results);
  console.log(`\nOpenRouter endpoint matrix summary\n${summary}\n`);

  const failures = results.filter((record) => record.result === "fail");
  expect(
    failures,
    `endpoint matrix failures detected\n${summary}`,
  ).toEqual([]);
});
