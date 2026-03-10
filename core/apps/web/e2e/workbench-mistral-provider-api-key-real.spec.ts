import type { APIRequestContext } from "playwright/test";
import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { test, expect } from "./fixtures";
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
const MISTRAL_BASE_URL = "https://api.mistral.ai/v1";
const MISTRAL_ENDPOINT_NAME = "Mistral Provider API Key E2E";

const selectedEndpointForConfig = (config: Record<string, unknown>): Record<string, unknown> | null => {
  const selectedEndpointId = readString(config.selected_endpoint_id);
  if (!selectedEndpointId) return null;
  return asArray(config.endpoints)
    .map((entry) => asRecord(entry))
    .find((entry) => readString(entry.id) === selectedEndpointId) ?? null;
};

async function readMistralHarnessConfig(request: APIRequestContext): Promise<Record<string, unknown>> {
  const configResp = await request.get("/api/providers/mistral/harness_config", {
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(configResp.ok(), `harness config read failed (${configResp.status()})`).toBe(true);
  return asRecord(await configResp.json());
}

async function ensureMistralEndpointSelected(
  request: APIRequestContext,
  apiKey: string,
): Promise<Record<string, unknown>> {
  let config = await readMistralHarnessConfig(request);
  let selectedEndpoint = selectedEndpointForConfig(config);
  const endpointMatches = selectedEndpoint !== null
    && readString(selectedEndpoint.name) === MISTRAL_ENDPOINT_NAME
    && readString(selectedEndpoint.base_url) === MISTRAL_BASE_URL
    && readString(selectedEndpoint.auth_type) === "bearer";
  if (readString(config.selected_source_kind) === "endpoint" && endpointMatches) {
    return config;
  }

  const upsertResp = await request.post("/api/providers/mistral/harness_config/endpoints", {
    data: {
      name: MISTRAL_ENDPOINT_NAME,
      base_url: MISTRAL_BASE_URL,
      api_shape: "openai_responses",
      auth_type: "bearer",
      api_key: apiKey,
      model_override: "",
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(upsertResp.ok(), `mistral endpoint upsert failed (${upsertResp.status()})`).toBe(true);
  config = asRecord(await upsertResp.json());
  selectedEndpoint =
    asArray(config.endpoints)
      .map((entry) => asRecord(entry))
      .find((entry) =>
        readString(entry.name) === MISTRAL_ENDPOINT_NAME
        && readString(entry.base_url) === MISTRAL_BASE_URL,
      ) ?? selectedEndpointForConfig(config);
  const endpointId = readString(selectedEndpoint?.id);
  expect(endpointId).not.toBe("");

  const selectResp = await request.post("/api/providers/mistral/harness_config/select", {
    data: {
      source_kind: "endpoint",
      endpoint_id: endpointId,
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(selectResp.ok(), `mistral endpoint select failed (${selectResp.status()})`).toBe(true);
  return asRecord(await selectResp.json());
}

test("workbench: mistral provider API key can run a real task", async ({ page, request }) => {
  test.setTimeout(10 * 60_000);

  if ((process.env.CTX_E2E_TIER ?? "") !== "provider-api-auth") {
    test.skip(true, "set CTX_E2E_TIER=provider-api-auth to run mistral provider auth e2e");
  }

  const mistralApiKey = (process.env.MISTRAL_API_KEY ?? "").trim();
  if (!mistralApiKey) {
    test.skip(true, "missing MISTRAL_API_KEY");
  }

  await ensureProviderInstalledAndHealthy(request, "mistral", INSTALL_TARGET, {
    timeoutMs: 10 * 60_000,
    pollMs: 2_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-mistral-provider-key-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "mistral provider auth e2e\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request,
    repo,
    workspaceName: `mistral-provider-auth-${Date.now()}`,
  });

  const config = await ensureMistralEndpointSelected(request, mistralApiKey);
  expect(readString(config.selected_source_kind)).toBe("endpoint");
  const selectedEndpoint = selectedEndpointForConfig(config);
  expect(selectedEndpoint).not.toBeNull();
  expect(readString(selectedEndpoint?.auth_type)).toBe("bearer");
  expect(readString(selectedEndpoint?.base_url)).toBe(MISTRAL_BASE_URL);
  expect(readString(selectedEndpoint?.name)).toBe(MISTRAL_ENDPOINT_NAME);

  await verifyProviderForWorkspace(request, workspaceId, "mistral", {
    timeoutMs: 90_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });

  const modelId = (process.env.CTX_E2E_MISTRAL_MODEL_ID ?? "").trim()
    || await resolveWorkspaceProviderModelId(request, workspaceId, "mistral", {
      timeoutMs: 90_000,
      pollMs: 3_000,
      requestTimeoutMs: REQUEST_TIMEOUT_MS,
    });

  const promptMarker = `mistral-provider-auth-${Date.now()}`;
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
      provider_id: "mistral",
      model_id: modelId,
      env_target: "worktree",
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
  expect(terminal.terminalStatus, terminal.errorMessage ?? "mistral run did not complete").toBe("completed");
  expect(terminal.assistantMessages).toBeGreaterThan(0);
});
