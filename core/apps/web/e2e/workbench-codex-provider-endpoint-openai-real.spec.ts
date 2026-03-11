import type { APIRequestContext } from "playwright/test";
import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { test, expect } from "./fixtures";
import { configureHarnessEndpointAuthViaModal } from "./utils/harnessEndpointAuth";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";
import {
  asArray,
  asRecord,
  ensureProviderInstalledAndHealthy,
  readString,
  resolveWorkspaceProviderModelId,
  verifyProviderForWorkspace,
  waitForTerminalState,
} from "../src/testing/providerRuntime";

const REQUEST_TIMEOUT_MS = 60_000;
const INSTALL_TARGET = "host" as const;
const CODEX_ENTRY = {
  providerId: "codex",
  menuLabel: "Codex",
  searchTerm: "codex",
};
const OPENAI_BASE_URL = "https://api.openai.com/v1";
const CODEX_OPENAI_ENDPOINT_NAME = "Codex OpenAI E2E";

const selectedEndpointForConfig = (config: Record<string, unknown>): Record<string, unknown> | null => {
  const selectedEndpointId = readString(config.selected_endpoint_id);
  if (!selectedEndpointId) return null;
  return asArray(config.endpoints)
    .map((entry) => asRecord(entry))
    .find((entry) => readString(entry.id) === selectedEndpointId) ?? null;
};

const authModalAlreadyConfigured = (detail: string): boolean =>
  detail.toLowerCase().includes("auth modal did not open (provider may already be configured)");

async function readCodexHarnessConfig(request: APIRequestContext): Promise<Record<string, unknown>> {
  const configResp = await request.get("/api/providers/codex/harness_config", {
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(configResp.ok(), `harness config read failed (${configResp.status()})`).toBe(true);
  return asRecord(await configResp.json());
}

async function ensureCodexOpenAiEndpointSelected(
  request: APIRequestContext,
  apiKey: string,
  modelOverride: string,
): Promise<Record<string, unknown>> {
  let config = await readCodexHarnessConfig(request);
  let selectedEndpoint = selectedEndpointForConfig(config);
  const endpointMatches = selectedEndpoint !== null
    && readString(selectedEndpoint.name) === CODEX_OPENAI_ENDPOINT_NAME
    && readString(selectedEndpoint.base_url) === OPENAI_BASE_URL
    && readString(selectedEndpoint.auth_type) === "bearer"
    && readString(selectedEndpoint.model_override) === modelOverride;
  if (readString(config.selected_source_kind) === "endpoint" && endpointMatches) {
    return config;
  }

  const upsertResp = await request.post("/api/providers/codex/harness_config/endpoints", {
    data: {
      name: CODEX_OPENAI_ENDPOINT_NAME,
      base_url: OPENAI_BASE_URL,
      api_shape: "openai_responses",
      auth_type: "bearer",
      api_key: apiKey,
      model_override: modelOverride,
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(upsertResp.ok(), `codex endpoint upsert failed (${upsertResp.status()})`).toBe(true);
  config = asRecord(await upsertResp.json());
  selectedEndpoint =
    asArray(config.endpoints)
      .map((entry) => asRecord(entry))
      .find((entry) =>
        readString(entry.name) === CODEX_OPENAI_ENDPOINT_NAME
        && readString(entry.base_url) === OPENAI_BASE_URL,
      ) ?? selectedEndpointForConfig(config);
  const endpointId = readString(selectedEndpoint?.id);
  expect(endpointId).not.toBe("");

  const selectResp = await request.post("/api/providers/codex/harness_config/select", {
    data: {
      source_kind: "endpoint",
      endpoint_id: endpointId,
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(selectResp.ok(), `codex endpoint select failed (${selectResp.status()})`).toBe(true);
  return asRecord(await selectResp.json());
}

test("workbench: codex OpenAI endpoint auth can run a real task", async ({ page, request }) => {
  test.setTimeout(10 * 60_000);

  if ((process.env.CTX_E2E_TIER ?? "") !== "provider-api-auth") {
    test.skip(true, "set CTX_E2E_TIER=provider-api-auth to run codex OpenAI provider-endpoint e2e");
  }

  const openAiApiKey = (process.env.OPENAI_API_KEY ?? "").trim();
  if (!openAiApiKey) {
    test.skip(true, "missing OPENAI_API_KEY");
  }

  const modelOverride = (process.env.CTX_E2E_CODEX_OPENAI_MODEL_OVERRIDE ?? "gpt-4.1").trim();

  await ensureProviderInstalledAndHealthy(request, "codex", INSTALL_TARGET, {
    timeoutMs: 10 * 60_000,
    pollMs: 2_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-codex-openai-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "codex openai provider endpoint e2e\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request,
    repo,
    workspaceName: `codex-openai-auth-${Date.now()}`,
  });

  const authResult = await configureHarnessEndpointAuthViaModal(
    page,
    CODEX_ENTRY,
    openAiApiKey,
    OPENAI_BASE_URL,
    modelOverride,
    {
      providerPresetLabel: "OpenAI",
      allowGenericProviderFallback: false,
      endpointName: CODEX_OPENAI_ENDPOINT_NAME,
    },
  );
  expect(authResult.ok || authModalAlreadyConfigured(authResult.detail), authResult.detail).toBe(true);

  const config = await ensureCodexOpenAiEndpointSelected(request, openAiApiKey, modelOverride);
  expect(readString(config.selected_source_kind)).toBe("endpoint");
  const selectedEndpoint = selectedEndpointForConfig(config);
  expect(selectedEndpoint).not.toBeNull();
  expect(readString(selectedEndpoint?.auth_type)).toBe("bearer");
  expect(readString(selectedEndpoint?.base_url)).toBe(OPENAI_BASE_URL);
  expect(readString(selectedEndpoint?.name)).toBe(CODEX_OPENAI_ENDPOINT_NAME);
  expect(readString(selectedEndpoint?.model_override)).toBe(modelOverride);

  await verifyProviderForWorkspace(request, workspaceId, "codex", {
    timeoutMs: 90_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });

  const modelId = await resolveWorkspaceProviderModelId(request, workspaceId, "codex", {
    timeoutMs: 90_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });

  const promptMarker = `codex-openai-provider-endpoint-${Date.now()}`;
  const prompt = `${promptMarker}: reply with exactly the word pong`;

  const createTaskResp = await request.post(`/api/workspaces/${workspaceId}/tasks`, {
    data: {
      title: promptMarker,
      create_default_session: false,
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(createTaskResp.ok(), `task create failed (${createTaskResp.status()})`).toBe(true);
  const taskId = readString(asRecord(await createTaskResp.json()).id);
  expect(taskId).not.toBe("");

  const createSessionResp = await request.post(`/api/tasks/${taskId}/sessions`, {
    data: {
      provider_id: "codex",
      model_id: modelId,
      execution_environment: "host",
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(createSessionResp.ok(), `session create failed (${createSessionResp.status()})`).toBe(true);
  const sessionId = readString(asRecord(await createSessionResp.json()).id);
  expect(sessionId).not.toBe("");

  const messageResp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content: prompt,
      delivery: "immediate",
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(messageResp.ok(), `message send failed (${messageResp.status()})`).toBe(true);

  const terminal = await waitForTerminalState(request, sessionId, {
    timeoutMs: 180_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });
  expect(terminal.terminalStatus, terminal.errorMessage ?? "codex OpenAI run did not complete").toBe("completed");
  expect(terminal.assistantMessages).toBeGreaterThan(0);
});
