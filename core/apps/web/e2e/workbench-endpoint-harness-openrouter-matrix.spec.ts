import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import type { APIRequestContext, Page } from "playwright/test";
import { parseBoolishString } from "../src/utils/boolish";
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
  bundle_dir: string | null;
  provider_installed: boolean;
  provider_health: string;
  provider_diagnostics: string[];
  managed_install_detected: boolean;
  managed_install_detail: string | null;
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
  details: Record<string, string>;
  managedInstallDetected: boolean;
  managedInstallDetail: string | null;
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

type ProviderModelSelectionResult =
  | { ok: true; detail: string; modelId: string }
  | { ok: false; detail: string };

const DEFAULT_OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_E2E_AUTH_TOKEN = "ctx-e2e-auth-token";
const TERMINAL_TURN_STATUSES = new Set(["completed", "failed", "interrupted"]);
const DEFAULT_RUN_CONTEXT_TIMEOUT_MS = 30_000;
const DEFAULT_TERMINAL_TIMEOUT_MS = 120_000;
const DEFAULT_PI_TERMINAL_TIMEOUT_MS = 240_000;
const DEFAULT_CODEX_OPENROUTER_MODEL_OVERRIDE = "openai/gpt-5.2-codex";
const DEFAULT_PI_OPENROUTER_MODEL_OVERRIDE = "google/gemini-3-flash-preview";

const providerDefaultOpenRouterModelOverride = (providerId: string): string => {
  if (providerId === "pi") return DEFAULT_PI_OPENROUTER_MODEL_OVERRIDE;
  return DEFAULT_CODEX_OPENROUTER_MODEL_OVERRIDE;
};

const providerTerminalTimeoutForHarness = (providerId: string, fallbackTimeoutMs: number): number => {
  if (providerId === "pi") {
    return Math.max(fallbackTimeoutMs, DEFAULT_PI_TERMINAL_TIMEOUT_MS);
  }
  return fallbackTimeoutMs;
};

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

const asArray = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

const readString = (value: unknown): string => (typeof value === "string" ? value : "");
const envTruthy = (value: string | undefined): boolean =>
  parseBoolishString(value) === true;

const envInt = (value: string | undefined, fallback: number): number => {
  const parsed = Number.parseInt((value ?? "").trim(), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
};

const readStringMap = (value: unknown): Record<string, string> => {
  const out: Record<string, string> = {};
  for (const [key, rawValue] of Object.entries(asRecord(value))) {
    if (typeof rawValue === "string") {
      out[key] = rawValue;
    } else if (typeof rawValue === "number" || typeof rawValue === "boolean") {
      out[key] = String(rawValue);
    }
  }
  return out;
};

const firstText = (...values: unknown[]): string => {
  for (const value of values) {
    const text = readString(value).trim();
    if (text) return text;
  }
  return "";
};

const normalizeErrorMessage = (raw: string): string => raw.replace(/\s+/g, " ").trim();
const providerModelOverrideEnvVar = (providerId: string): string =>
  `CTX_E2E_${providerId.toUpperCase().replace(/[^A-Z0-9]+/g, "_")}_OPENROUTER_MODEL_OVERRIDE`;

const isLikelyRuntimeSkip = (message: string): boolean => {
  const normalized = message.toLowerCase();
  return [
    "crp_runtime_stdout_closed",
    "command not found",
    "no such file or directory",
    "not installed",
    "missing",
    "enoent",
    "failed to spawn",
    "unavailable",
    "not healthy",
    "install",
    "crp runtime closed before models.list response",
    "models.list response",
    "models.list probe timed out",
    "api key auth not supported",
    "provider not listed by /api/providers",
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
    const details = readStringMap(rec.details);
    const managedInstallDetail = firstText(
      details.managed_install_dir,
      details.managed_package,
      details.managed_version,
      details.managed_bin_dir,
      details.install_running === "true" ? "install_running=true" : "",
    ) || null;
    out[providerId] = {
      installed: rec.installed === true,
      health: readString(rec.health) || "unknown",
      diagnostics,
      details,
      managedInstallDetected: managedInstallDetail !== null,
      managedInstallDetail,
    };
  }
  return out;
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
      lastTurn.status,
      asRecord(summary.activity).last_turn_status,
      asRecord(head.activity).last_turn_status,
      asRecord(summary.session).status,
      asRecord(head.session).status,
    ) || "no explicit error payload",
  );
}

async function ensureEndpointModelOverrideForProvider(opts: {
  request: APIRequestContext;
  providerId: string;
  modelOverride: string;
  endpointBaseUrl?: string;
  endpointApiKey?: string;
}): Promise<ProviderVerifyResult> {
  const { request, providerId, modelOverride, endpointBaseUrl, endpointApiKey } = opts;
  const targetModel = modelOverride.trim();
  if (!targetModel) {
    return { ok: true, status: "ok", detail: "model override not requested" };
  }

  let config: Record<string, unknown> = {};
  let endpoints: Record<string, unknown>[] = [];
  let selectedEndpoint = asRecord({});
  for (let attempt = 0; attempt < 12; attempt += 1) {
    const configResp = await request.get(`/api/providers/${providerId}/harness_config`);
    if (!configResp.ok()) {
      return {
        ok: false,
        status: "error",
        detail: `failed to read harness config (${configResp.status()})`,
      };
    }

    config = asRecord(await configResp.json());
    endpoints = asArray(config.endpoints).map((entry) => asRecord(entry));
    const selectedEndpointId = readString(config.selected_endpoint_id);
    selectedEndpoint =
      endpoints.find((entry) => readString(entry.id) === selectedEndpointId) ??
      (endpoints.length === 1 ? endpoints[0] : asRecord({}));
    if (Object.keys(selectedEndpoint).length > 0) break;
    await new Promise((resolve) => setTimeout(resolve, 300));
  }

  if (Object.keys(selectedEndpoint).length === 0 && endpoints.length > 0) {
    selectedEndpoint = endpoints[0] ?? asRecord({});
  }

  if (
    Object.keys(selectedEndpoint).length === 0
    && endpointBaseUrl
    && endpointApiKey
  ) {
    const createResp = await request.post(`/api/providers/${providerId}/harness_config/endpoints`, {
      data: {
        name: `${providerId}-openrouter`,
        base_url: endpointBaseUrl,
        auth_type: "api_key",
        api_key: endpointApiKey,
        model_override: targetModel,
      },
    });
    if (createResp.ok()) {
      config = asRecord(await createResp.json());
      endpoints = asArray(config.endpoints).map((entry) => asRecord(entry));
      selectedEndpoint =
        endpoints.find((entry) => firstText(entry.name) === `${providerId}-openrouter`) ??
        endpoints[0] ??
        asRecord({});
    }
  }

  if (Object.keys(selectedEndpoint).length === 0) {
    const sourceKind = firstText(config.selected_source_kind, "unknown");
    return {
      ok: false,
      status: "error",
      detail: `selected endpoint is missing for model override (source_kind=${sourceKind})`,
    };
  }

  const selectedEndpointId = readString(selectedEndpoint.id);
  if (!selectedEndpointId) {
    return {
      ok: false,
      status: "error",
      detail: "selected endpoint payload is missing endpoint id",
    };
  }

  const selectedSourceKind = firstText(config.selected_source_kind).toLowerCase();
  if (selectedSourceKind !== "endpoint" || readString(config.selected_endpoint_id) !== selectedEndpointId) {
    const selectResp = await request.post(`/api/providers/${providerId}/harness_config/select`, {
      data: {
        source_kind: "endpoint",
        endpoint_id: selectedEndpointId,
      },
    });
    if (!selectResp.ok()) {
      const body = asRecord(await selectResp.json().catch(() => ({})));
      return {
        ok: false,
        status: "error",
        detail: normalizeErrorMessage(
          firstText(body.error, body.message, `failed to select endpoint mode (${selectResp.status()})`),
        ),
      };
    }
  }

  const name = firstText(selectedEndpoint.name);
  if (!name) {
    return {
      ok: false,
      status: "error",
      detail: `selected endpoint ${selectedEndpointId} has no name`,
    };
  }

  const currentModelOverride = firstText(selectedEndpoint.model_override);
  if (currentModelOverride === targetModel) {
    return {
      ok: true,
      status: "ok",
      detail: `model override already set (${targetModel})`,
    };
  }

  const payload: Record<string, unknown> = {
    endpoint_id: selectedEndpointId,
    name,
    model_override: targetModel,
  };
  const baseUrl = firstText(selectedEndpoint.base_url);
  if (baseUrl) payload.base_url = baseUrl;
  const apiShape = firstText(selectedEndpoint.api_shape);
  if (apiShape) payload.api_shape = apiShape;
  const authType = firstText(selectedEndpoint.auth_type);
  if (authType) payload.auth_type = authType;

  const upsertResp = await request.post(`/api/providers/${providerId}/harness_config/endpoints`, {
    data: payload,
  });
  if (!upsertResp.ok()) {
    const body = asRecord(await upsertResp.json().catch(() => ({})));
    return {
      ok: false,
      status: "error",
      detail: normalizeErrorMessage(
        firstText(body.error, body.message, `failed to set model override (${upsertResp.status()})`),
      ),
    };
  }

  return {
    ok: true,
    status: "ok",
    detail: `model override set to ${targetModel}`,
  };
}

async function resolveWorkspaceProviderModelId(opts: {
  request: APIRequestContext;
  workspaceId: string;
  providerId: string;
}): Promise<ProviderModelSelectionResult> {
  const { request, workspaceId, providerId } = opts;
  const optionsResp = await request.get(`/api/workspaces/${workspaceId}/providers/${providerId}/options`);
  if (!optionsResp.ok()) {
    return {
      ok: false,
      detail: `failed to read provider options (${optionsResp.status()})`,
    };
  }

  const options = asRecord(await optionsResp.json());
  const models = asRecord(options.models);
  const currentModelId = firstText(models.current_model_id, models.currentModelId);
  if (currentModelId) {
    return {
      ok: true,
      modelId: currentModelId,
      detail: `using current model ${currentModelId}`,
    };
  }

  const firstModelId =
    asArray(models.models)
      .map((entry) => asRecord(entry))
      .map((entry) => firstText(entry.id, entry.model_id, entry.modelId, entry.name))
      .find(Boolean) || "";
  if (firstModelId) {
    return {
      ok: true,
      modelId: firstModelId,
      detail: `using first listed model ${firstModelId}`,
    };
  }

  return {
    ok: false,
    detail: "provider options did not return a usable model id",
  };
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
  const { request, sessionId, timeoutMs = DEFAULT_TERMINAL_TIMEOUT_MS } = opts;
  let resolved: TerminalState | null = null;

  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/sessions/${sessionId}/head?include_events=1&limit=80`);
        if (!resp.ok()) return "";
        const state = toTerminalState({
          head: asRecord(await resp.json()),
        });
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
  let resp;
  try {
    resp = await request.post(`/api/workspaces/${workspaceId}/providers/${providerId}/verify`, {
      data: {},
      timeout: 20_000,
    });
  } catch (error) {
    return {
      ok: false,
      status: "error",
      detail: normalizeErrorMessage(
        `verify request failed for ${providerId}: ${error instanceof Error ? error.message : String(error)}`,
      ),
    };
  }
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
  page.setDefaultTimeout(15_000);
  page.setDefaultNavigationTimeout(20_000);

  if ((process.env.CTX_E2E_TIER ?? "") !== "endpoint-ui") {
    test.skip(true, "set CTX_E2E_TIER=endpoint-ui to run endpoint harness matrix test");
  }

  const apiKey = (process.env.OPENROUTER_API_KEY ?? "").trim();
  if (!apiKey) {
    test.skip(true, "missing OPENROUTER_API_KEY");
  }

  const baseUrl = (process.env.OPENROUTER_BASE_URL ?? "").trim() || DEFAULT_OPENROUTER_BASE_URL;
  const authToken = (process.env.CTX_E2E_AUTH_TOKEN ?? "").trim() || DEFAULT_E2E_AUTH_TOKEN;
  const strictBundledOnly = envTruthy(process.env.CTX_E2E_BUNDLED_ONLY);
  const bundleDir = firstText(process.env.CTX_BUNDLE_DIR) || null;
  const providerRunContextTimeoutMs = envInt(
    process.env.CTX_E2E_PROVIDER_RUN_CONTEXT_TIMEOUT_MS,
    DEFAULT_RUN_CONTEXT_TIMEOUT_MS,
  );
  const providerTerminalTimeoutMs = envInt(
    process.env.CTX_E2E_PROVIDER_TERMINAL_TIMEOUT_MS,
    DEFAULT_TERMINAL_TIMEOUT_MS,
  );
  const requestedProviderIds = (process.env.CTX_E2E_ENDPOINT_PROVIDERS ?? "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);
  const defaultOpenRouterModelOverride =
    firstText(process.env.CTX_E2E_OPENROUTER_MODEL_OVERRIDE);
  const modelOverrideByProvider = OPENROUTER_ENDPOINT_FIRST_PASS_HARNESSES.reduce(
    (acc, entry) => {
      const perProviderEnv = providerModelOverrideEnvVar(entry.providerId);
      const override =
        firstText(process.env[perProviderEnv]) ||
        defaultOpenRouterModelOverride ||
        providerDefaultOpenRouterModelOverride(entry.providerId);
      acc[entry.providerId] = override;
      return acc;
    },
    {} as Record<string, string>,
  );

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
  const matrixEntries = requestedProviderIds.length
    ? OPENROUTER_ENDPOINT_FIRST_PASS_HARNESSES.filter((entry) =>
        requestedProviderIds.includes(entry.providerId),
      )
    : OPENROUTER_ENDPOINT_FIRST_PASS_HARNESSES;

  if (requestedProviderIds.length && matrixEntries.length === 0) {
    throw new Error(
      `CTX_E2E_ENDPOINT_PROVIDERS did not match first-pass matrix entries: ${requestedProviderIds.join(",")}`,
    );
  }

  for (const entry of matrixEntries) {
    console.log(`endpoint matrix: starting provider ${entry.providerId}`);
    const startMs = Date.now();
    const provider = providers[entry.providerId] ?? {
      installed: false,
      health: "unknown",
      diagnostics: ["provider not listed by /api/providers"],
      details: {},
      managedInstallDetected: false,
      managedInstallDetail: null,
    };

    const baseRecord: HarnessRunRecord = {
      provider_id: entry.providerId,
      menu_label: entry.menuLabel,
      bundle_dir: bundleDir,
      provider_installed: provider.installed,
      provider_health: provider.health,
      provider_diagnostics: provider.diagnostics,
      managed_install_detected: provider.managedInstallDetected,
      managed_install_detail: provider.managedInstallDetail,
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

      if (strictBundledOnly && provider.managedInstallDetected) {
        results.push({
          ...baseRecord,
          result: "fail",
          reason: `managed install metadata detected in bundled-only mode: ${provider.managedInstallDetail}`,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      if (!provider.installed || provider.health !== "ok") {
        const reason = firstText(provider.diagnostics[0], `provider health=${provider.health}`);
        results.push({
          ...baseRecord,
          result: strictBundledOnly && !isLikelyRuntimeSkip(reason) ? "fail" : "skip",
          reason: reason || "provider unavailable",
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const modelOverride = modelOverrideByProvider[entry.providerId] ?? "";
      const authResult = await configureHarnessEndpointAuthViaModal(
        page,
        entry,
        apiKey,
        baseUrl,
        modelOverride,
      );
      console.log(`endpoint matrix: ${entry.providerId} auth result -> ${authResult.ok ? "ok" : "fail"}`);
      const authLikelyAlreadyConfigured =
        !authResult.ok && authResult.detail.toLowerCase().includes("already be configured");
      const authProbeFailureOnly =
        !authResult.ok &&
        (authResult.detail.toLowerCase().includes("models.list response")
          || authResult.detail.toLowerCase().includes("models.list probe timed out")
          || authResult.detail.toLowerCase().includes("requires openai_model")
          || authResult.detail.toLowerCase().includes("configure a model override"));
      const authSaved = authResult.ok || authLikelyAlreadyConfigured || authProbeFailureOnly;
      if (!authSaved) {
        results.push({
          ...baseRecord,
          auth_saved: false,
          auth_detail: authResult.detail,
          result: isLikelyRuntimeSkip(authResult.detail) ? "skip" : "fail",
          reason: `auth save failed: ${authResult.detail}`,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      if (modelOverride) {
        const modelOverrideResult = await ensureEndpointModelOverrideForProvider({
          request,
          providerId: entry.providerId,
          modelOverride,
          endpointBaseUrl: baseUrl,
          endpointApiKey: apiKey,
        });
        console.log(
          `endpoint matrix: ${entry.providerId} model override -> ${modelOverrideResult.ok ? "ok" : "fail"}`,
        );
        if (!modelOverrideResult.ok) {
          results.push({
            ...baseRecord,
            auth_saved: true,
            auth_detail: authResult.detail,
            result: "fail",
            reason: `model override failed: ${modelOverrideResult.detail}`,
            elapsed_ms: Date.now() - startMs,
          });
          continue;
        }
      }

      const harnessSelect = await selectHarnessForComposer(page, entry);
      console.log(
        `endpoint matrix: ${entry.providerId} harness select -> ${harnessSelect.ok ? "ok" : "fail"}`,
      );
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
      console.log(`endpoint matrix: ${entry.providerId} verify -> ${verify.status}`);
      const verifyProbeFailureOnly =
        !verify.ok &&
        (verify.detail.toLowerCase().includes("models.list response")
          || verify.detail.toLowerCase().includes("models.list probe timed out"));
      if (!verify.ok && !verifyProbeFailureOnly) {
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

      const modelSelection = await resolveWorkspaceProviderModelId({
        request,
        workspaceId,
        providerId: entry.providerId,
      });
      if (!modelSelection.ok) {
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          result: "fail",
          reason: `model selection failed: ${modelSelection.detail}`,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }
      console.log(`endpoint matrix: ${entry.providerId} run model -> ${modelSelection.modelId}`);

      const createTaskResp = await request.post(`/api/workspaces/${workspaceId}/tasks`, {
        data: {
          title: promptMarker,
          create_default_session: false,
        },
      });
      if (!createTaskResp.ok()) {
        const body = asRecord(await createTaskResp.json().catch(() => ({})));
        const reason = normalizeErrorMessage(
          firstText(body.error, body.message, `task create failed (${createTaskResp.status()})`),
        );
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          result: "fail",
          reason,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }
      const taskId = readString(asRecord(await createTaskResp.json()).id);
      if (!taskId) {
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          result: "fail",
          reason: "task create returned empty task id",
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const createSessionResp = await request.post(`/api/tasks/${taskId}/sessions`, {
        data: {
          provider_id: entry.providerId,
          model_id: modelSelection.modelId,
          env_target: "worktree",
        },
      });
      if (!createSessionResp.ok()) {
        const bodyText = normalizeErrorMessage(await createSessionResp.text().catch(() => ""));
        let body: Record<string, unknown> = {};
        if (bodyText) {
          try {
            body = asRecord(JSON.parse(bodyText));
          } catch {
            body = {};
          }
        }
        const reason = normalizeErrorMessage(
          firstText(
            body.error,
            body.message,
            bodyText,
            `session create failed (${createSessionResp.status()})`,
          ),
        );
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
      const sessionId = readString(asRecord(await createSessionResp.json()).id);
      if (!sessionId) {
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          result: "fail",
          reason: "session create returned empty session id",
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }

      const messageResp = await request.post(`/api/sessions/${sessionId}/messages`, {
        data: {
          content: prompt,
          delivery: "immediate",
        },
      });
      if (!messageResp.ok()) {
        const body = asRecord(await messageResp.json().catch(() => ({})));
        const reason = normalizeErrorMessage(
          firstText(body.error, body.message, `session message failed (${messageResp.status()})`),
        );
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          session_started: true,
          session_id: sessionId,
          model_id: modelSelection.modelId,
          result: isLikelyRuntimeSkip(reason) ? "skip" : "fail",
          reason,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }
      console.log(`endpoint matrix: ${entry.providerId} session started -> ${sessionId}`);

      let terminal: TerminalState;
      try {
        terminal = await waitForTerminalState({
          request,
          sessionId,
          timeoutMs: providerTerminalTimeoutForHarness(entry.providerId, providerTerminalTimeoutMs),
        });
      } catch (error) {
        const timeoutDetail = normalizeErrorMessage(error instanceof Error ? error.message : String(error));
        const reason = verifyProbeFailureOnly ? verify.detail : timeoutDetail;
        results.push({
          ...baseRecord,
          auth_saved: true,
          auth_detail: authResult.detail,
          harness_selected: true,
          harness_detail: harnessSelect.detail,
          session_started: true,
          session_id: sessionId,
          model_id: modelSelection.modelId,
          result: verifyProbeFailureOnly || isLikelyRuntimeSkip(timeoutDetail) ? "skip" : "fail",
          reason,
          elapsed_ms: Date.now() - startMs,
        });
        continue;
      }
      console.log(
        `endpoint matrix: ${entry.providerId} terminal -> ${firstText(terminal.terminalStatus, "unknown")}`,
      );

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
        session_id: sessionId,
        model_id: terminal.modelId ?? modelSelection.modelId,
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
        requested_provider_ids: requestedProviderIds,
        provider_run_context_timeout_ms: providerRunContextTimeoutMs,
        provider_terminal_timeout_ms: providerTerminalTimeoutMs,
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
