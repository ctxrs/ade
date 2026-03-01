import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import type { APIRequestContext } from "playwright/test";

type ProviderStatus = {
  installed: boolean;
  health: string;
  diagnostics: string[];
  details: Record<string, string>;
};

type TerminalState = {
  done: boolean;
  terminalStatus: string | null;
  assistantMessages: number;
  errorMessage: string | null;
  modelId: string | null;
};

const DEFAULT_OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_PROVIDER_ID = "codex";
const DEFAULT_MODEL_OVERRIDE = "openai/gpt-5.2-codex";
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

const readStringMap = (value: unknown): Record<string, string> => {
  const out: Record<string, string> = {};
  for (const [key, rawValue] of Object.entries(asRecord(value))) {
    if (typeof rawValue === "string") {
      out[key] = rawValue;
      continue;
    }
    if (typeof rawValue === "number" || typeof rawValue === "boolean") {
      out[key] = String(rawValue);
    }
  }
  return out;
};

const initRepo = (): string => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "runtime provider install openrouter smoke e2e\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });
  return repo;
};

async function getProviderStatus(request: APIRequestContext, providerId: string): Promise<ProviderStatus> {
  const response = await request.get("/api/providers");
  expect(response.ok(), `failed to read providers (${response.status()})`).toBeTruthy();
  const rows = asArray(await response.json()).map((entry) => asRecord(entry));
  const row = rows.find((entry) => readString(entry.provider_id) === providerId);
  if (!row) {
    return {
      installed: false,
      health: "missing",
      diagnostics: ["provider not listed by /api/providers"],
      details: {},
    };
  }
  return {
    installed: row.installed === true,
    health: firstText(row.health, "unknown"),
    diagnostics: asArray(row.diagnostics).map((entry) => readString(entry)).filter(Boolean),
    details: readStringMap(row.details),
  };
}

async function installProviderAndWait(
  request: APIRequestContext,
  providerId: string,
  target: "host" | "container",
): Promise<string> {
  const start = await request.post(`/api/providers/${providerId}/install?target=${target}`, { data: {} });
  if (!start.ok()) {
    const body = normalizeErrorMessage(await start.text().catch(() => ""));
    throw new Error(`provider install start failed (${start.status()}): ${body}`);
  }
  const payload = asRecord(await start.json());
  const installId = readString(payload.install_id);
  if (!installId) {
    throw new Error(`provider install response missing install_id: ${JSON.stringify(payload)}`);
  }

  await expect
    .poll(
      async () => {
        const poll = await request.get(`/api/providers/install/${installId}`);
        if (!poll.ok()) {
          throw new Error(`provider install poll failed (${poll.status()})`);
        }
        const info = asRecord(await poll.json());
        const state = firstText(info.state).toLowerCase();
        if (state === "failed" || state === "cancelled") {
          const lastEvent = asRecord(info.last_event);
          const detail = normalizeErrorMessage(
            firstText(
              lastEvent.message,
              lastEvent.stage,
              info.error,
              JSON.stringify(info),
            ),
          );
          throw new Error(`provider install ${state}: ${detail}`);
        }
        return state;
      },
      { timeout: 10 * 60_000, intervals: [1_000, 2_000, 3_000] },
    )
    .toBe("succeeded");

  return installId;
}

async function configureOpenRouterEndpoint(
  request: APIRequestContext,
  providerId: string,
  baseUrl: string,
  apiKey: string,
  modelOverride: string,
): Promise<void> {
  const endpointName = `${providerId}-openrouter-smoke`;
  const upsert = await request.post(`/api/providers/${providerId}/harness_config/endpoints`, {
    data: {
      name: endpointName,
      base_url: baseUrl,
      auth_type: "api_key",
      api_key: apiKey,
      model_override: modelOverride,
    },
  });
  if (!upsert.ok()) {
    const body = normalizeErrorMessage(await upsert.text().catch(() => ""));
    throw new Error(`endpoint upsert failed (${upsert.status()}): ${body}`);
  }
  const config = asRecord(await upsert.json());
  const endpoints = asArray(config.endpoints).map((entry) => asRecord(entry));
  const chosen =
    endpoints.find((entry) => readString(entry.name) === endpointName)
    ?? endpoints.find((entry) => readString(entry.id) === readString(config.selected_endpoint_id))
    ?? asRecord({});
  const endpointId = firstText(chosen.id, config.selected_endpoint_id);
  if (!endpointId) {
    throw new Error(`endpoint upsert returned no endpoint id: ${JSON.stringify(config)}`);
  }
  const select = await request.post(`/api/providers/${providerId}/harness_config/select`, {
    data: {
      source_kind: "endpoint",
      endpoint_id: endpointId,
    },
  });
  if (!select.ok()) {
    const body = normalizeErrorMessage(await select.text().catch(() => ""));
    throw new Error(`endpoint select failed (${select.status()}): ${body}`);
  }
}

async function verifyProviderForWorkspace(
  request: APIRequestContext,
  workspaceId: string,
  providerId: string,
): Promise<void> {
  const response = await request.post(`/api/workspaces/${workspaceId}/providers/${providerId}/verify`, {
    data: {},
    timeout: 30_000,
  });
  if (!response.ok()) {
    const body = normalizeErrorMessage(await response.text().catch(() => ""));
    throw new Error(`provider verify request failed (${response.status()}): ${body}`);
  }
  const payload = asRecord(await response.json());
  const status = firstText(payload.status).toLowerCase();
  if (status !== "ok") {
    const detail = normalizeErrorMessage(firstText(payload.message, JSON.stringify(payload)));
    throw new Error(`provider verify failed (status=${status}): ${detail}`);
  }
}

async function resolveWorkspaceProviderModelId(
  request: APIRequestContext,
  workspaceId: string,
  providerId: string,
): Promise<string> {
  let resolved = "";
  await expect
    .poll(
      async () => {
        const response = await request.get(`/api/workspaces/${workspaceId}/providers/${providerId}/options`);
        if (!response.ok()) {
          return "";
        }
        const payload = asRecord(await response.json());
        const models = asRecord(payload.models);
        const currentModel = firstText(models.current_model_id, models.currentModelId);
        if (currentModel) {
          resolved = currentModel;
          return currentModel;
        }
        const firstModel =
          asArray(models.models)
            .map((entry) => asRecord(entry))
            .map((entry) => firstText(entry.id, entry.model_id, entry.modelId, entry.name))
            .find(Boolean) || "";
        resolved = firstModel;
        return firstModel;
      },
      { timeout: 60_000, intervals: [1_000, 2_000, 3_000] },
    )
    .not.toBe("");
  return resolved;
}

function extractErrorMessage(snapshot: Record<string, unknown>): string {
  const head = asRecord(snapshot.head);
  const turns = asArray(head.turns).map((entry) => asRecord(entry));
  const events = asArray(head.events).map((entry) => asRecord(entry));
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const event = events[i];
    const payload = asRecord(event.payload_json);
    const message = firstText(payload.message, payload.error, payload.reason, payload.detail, payload.stderr, payload.stdout);
    if (message) {
      const eventType = firstText(event.event_type);
      return normalizeErrorMessage(eventType ? `[${eventType}] ${message}` : message);
    }
  }
  const lastTurn = turns.length > 0 ? turns[turns.length - 1] : {};
  return normalizeErrorMessage(firstText(lastTurn.status, "no explicit error payload"));
}

function toTerminalState(snapshot: Record<string, unknown>): TerminalState {
  const head = asRecord(snapshot.head);
  const activity = asRecord(head.activity);
  const turns = asArray(head.turns).map((entry) => asRecord(entry));
  const messages = asArray(head.messages).map((entry) => asRecord(entry));
  const lastTurn = turns.length > 0 ? turns[turns.length - 1] : {};
  const terminalStatus = firstText(lastTurn.status, activity.last_turn_status).toLowerCase() || null;
  const assistantMessages = messages.filter((message) => {
    if (firstText(message.role) !== "assistant") return false;
    return firstText(message.content).length > 0;
  }).length;
  const headWorking = activity.is_working === true;
  const done = terminalStatus ? TERMINAL_TURN_STATUSES.has(terminalStatus) : !headWorking && assistantMessages > 0;
  const modelId = firstText(asRecord(head.session).model_id) || null;
  const failed = terminalStatus === "failed" || terminalStatus === "interrupted";
  return {
    done,
    terminalStatus,
    assistantMessages,
    errorMessage: failed ? extractErrorMessage(snapshot) : null,
    modelId,
  };
}

async function waitForTerminalState(
  request: APIRequestContext,
  sessionId: string,
  timeoutMs: number,
): Promise<TerminalState> {
  let resolved: TerminalState | null = null;
  await expect
    .poll(
      async () => {
        const response = await request.get(`/api/sessions/${sessionId}/head?include_events=1&limit=80`);
        if (!response.ok()) return "";
        const state = toTerminalState({ head: asRecord(await response.json()) });
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

test("runtime install smoke: fresh daemon can install provider and run OpenRouter prompt", async ({ request }) => {
  test.setTimeout(25 * 60_000);

  if ((process.env.CTX_E2E_TIER ?? "") !== "endpoint-ui") {
    test.skip(true, "set CTX_E2E_TIER=endpoint-ui to run runtime install OpenRouter smoke");
  }

  const apiKey = (process.env.OPENROUTER_API_KEY ?? "").trim();
  if (!apiKey) {
    test.skip(true, "missing OPENROUTER_API_KEY");
  }

  const providerId = (process.env.CTX_E2E_INSTALL_SMOKE_PROVIDER ?? DEFAULT_PROVIDER_ID).trim() || DEFAULT_PROVIDER_ID;
  const modelOverride =
    (process.env.CTX_E2E_INSTALL_SMOKE_MODEL_OVERRIDE ?? DEFAULT_MODEL_OVERRIDE).trim() || DEFAULT_MODEL_OVERRIDE;
  const baseUrl = (process.env.OPENROUTER_BASE_URL ?? "").trim() || DEFAULT_OPENROUTER_BASE_URL;

  const repo = initRepo();
  const workspaceResp = await request.post("/api/workspaces", {
    data: {
      root_path: repo,
      name: `runtime-install-smoke-${Date.now()}`,
    },
  });
  expect(workspaceResp.ok(), `workspace create failed (${workspaceResp.status()})`).toBeTruthy();
  const workspaceId = firstText(asRecord(await workspaceResp.json()).id);
  expect(workspaceId).not.toBe("");

  const providerBefore = await getProviderStatus(request, providerId);
  console.log(`install smoke: provider=${providerId} before installed=${providerBefore.installed} health=${providerBefore.health}`);

  const installId = await installProviderAndWait(request, providerId, "host");
  console.log(`install smoke: provider=${providerId} install_id=${installId} completed`);

  const providerAfter = await getProviderStatus(request, providerId);
  expect(providerAfter.installed).toBeTruthy();
  expect(providerAfter.health).toBe("ok");

  await configureOpenRouterEndpoint(request, providerId, baseUrl, apiKey, modelOverride);
  await verifyProviderForWorkspace(request, workspaceId, providerId);
  const modelId = await resolveWorkspaceProviderModelId(request, workspaceId, providerId);
  expect(modelId).not.toBe("");

  const taskResp = await request.post(`/api/workspaces/${workspaceId}/tasks`, {
    data: {
      title: `runtime-install-smoke-${Date.now()}`,
      create_default_session: false,
    },
  });
  expect(taskResp.ok(), `task create failed (${taskResp.status()})`).toBeTruthy();
  const taskId = firstText(asRecord(await taskResp.json()).id);
  expect(taskId).not.toBe("");

  const sessionResp = await request.post(`/api/tasks/${taskId}/sessions`, {
    data: {
      provider_id: providerId,
      model_id: modelId,
      env_target: "worktree",
    },
  });
  if (!sessionResp.ok()) {
    const body = normalizeErrorMessage(await sessionResp.text().catch(() => ""));
    throw new Error(`session create failed (${sessionResp.status()}): ${body}`);
  }
  const sessionId = firstText(asRecord(await sessionResp.json()).id);
  expect(sessionId).not.toBe("");

  const prompt = `runtime-install-smoke-${Date.now()}: reply with exactly the word pong`;
  const messageResp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content: prompt,
      delivery: "immediate",
    },
  });
  if (!messageResp.ok()) {
    const body = normalizeErrorMessage(await messageResp.text().catch(() => ""));
    throw new Error(`session message failed (${messageResp.status()}): ${body}`);
  }

  const terminal = await waitForTerminalState(request, sessionId, 180_000);
  if (terminal.terminalStatus !== "completed" || terminal.assistantMessages <= 0) {
    const detail = normalizeErrorMessage(
      firstText(terminal.errorMessage, `terminal_status=${terminal.terminalStatus}`, "assistant completion missing"),
    );
    throw new Error(`runtime install smoke session failed: ${detail}`);
  }
});
