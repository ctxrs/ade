const { daemonJson } = require("./daemon.cjs");
const { providerStatusPath } = require("../../../../test-support/provider_status_path.cjs");

const DEFAULT_OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_CODEX_OPENROUTER_MODEL_OVERRIDE = "openai/gpt-5.2-codex";

const asRecord = (value) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value;
};

const asArray = (value) => (Array.isArray(value) ? value : []);

const readString = (value) => (typeof value === "string" ? value : "");

const firstText = (...values) => {
  for (const value of values) {
    const text = readString(value).trim();
    if (text) return text;
  }
  return "";
};

const normalizeErrorMessage = (raw) => String(raw || "").replace(/\s+/g, " ").trim();

const readStringMap = (value) => {
  const out = {};
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

const getProviderStatus = async (providerId, target = "host") => {
  const response = await daemonJson("GET", providerStatusPath(providerId, target));
  if (response.status !== 200) {
    throw new Error(`failed to read providers (${response.status})`);
  }
  const row = asRecord(response.payload);
  if (!row) {
    return {
      installed: false,
      health: "missing",
      diagnostics: [`provider not returned by ${providerStatusPath(providerId, target)}`],
      details: {},
    };
  }
  return {
    installed: row.installed === true,
    health: firstText(row.health, "unknown"),
    diagnostics: asArray(row.diagnostics).map((entry) => readString(entry)).filter(Boolean),
    details: readStringMap(row.details),
  };
};

const installProviderAndWait = async (
  providerId,
  target,
  { timeoutMs = 10 * 60_000, pollMs = 2000 } = {},
) => {
  const start = await daemonJson("POST", `/api/providers/${providerId}/install?target=${target}`, {});
  if (start.status !== 200) {
    throw new Error(`provider install start failed (${start.status}): ${normalizeErrorMessage(JSON.stringify(start.payload))}`);
  }
  const payload = asRecord(start.payload);
  const installId = readString(payload.install_id);
  if (!installId) {
    throw new Error(`provider install response missing install_id: ${JSON.stringify(payload)}`);
  }

  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const poll = await daemonJson("GET", `/api/providers/install/${installId}`);
    if (poll.status !== 200) {
      throw new Error(`provider install poll failed (${poll.status})`);
    }
    const info = asRecord(poll.payload);
    const state = firstText(info.state).toLowerCase();
    if (state === "succeeded") return installId;
    if (state === "failed" || state === "cancelled") {
      const lastEvent = asRecord(info.last_event);
      const detail = normalizeErrorMessage(
        firstText(lastEvent.message, lastEvent.stage, info.error, JSON.stringify(info)),
      );
      throw new Error(`provider install ${state}: ${detail}`);
    }
    await browser.pause(pollMs);
  }
  throw new Error(`provider install timed out for ${providerId} (${installId})`);
};

const configureOpenRouterEndpoint = async ({
  providerId,
  baseUrl,
  apiKey,
  modelOverride = "",
  endpointName = "",
}) => {
  const name = endpointName || `${providerId}-openrouter-desktop-smoke`;
  const upsert = await daemonJson("POST", `/api/providers/${providerId}/harness_config/endpoints`, {
    name,
    base_url: baseUrl,
    auth_type: "api_key",
    api_key: apiKey,
    model_override: modelOverride,
  });
  if (upsert.status !== 200) {
    throw new Error(`endpoint upsert failed (${upsert.status}): ${normalizeErrorMessage(JSON.stringify(upsert.payload))}`);
  }
  const config = asRecord(upsert.payload);
  const endpoints = asArray(config.endpoints).map((entry) => asRecord(entry));
  const chosen = endpoints.find((entry) => readString(entry.name) === name)
    || endpoints.find((entry) => readString(entry.id) === readString(config.selected_endpoint_id))
    || {};
  const endpointId = firstText(chosen.id, config.selected_endpoint_id);
  if (!endpointId) {
    throw new Error(`endpoint upsert returned no endpoint id: ${JSON.stringify(config)}`);
  }

  await selectHarnessSource({
    providerId,
    sourceKind: "endpoint",
    endpointId,
  });
  return endpointId;
};

const selectHarnessSource = async ({
  providerId,
  sourceKind,
  endpointId = null,
}) => {
  const body = {
    source_kind: String(sourceKind || "").trim(),
  };
  if (endpointId) body.endpoint_id = endpointId;
  const select = await daemonJson("POST", `/api/providers/${providerId}/harness_config/select`, body);
  if (select.status !== 200) {
    throw new Error(
      `harness source select failed (${select.status}): ${normalizeErrorMessage(JSON.stringify(select.payload))}`,
    );
  }
  return asRecord(select.payload);
};

const selectSubscriptionSource = async (providerId) => {
  return await selectHarnessSource({
    providerId,
    sourceKind: "subscription",
  });
};

const refreshEndpointModels = async (providerId, endpointId) => {
  const response = await daemonJson(
    "POST",
    `/api/providers/${providerId}/harness_config/endpoints/${encodeURIComponent(endpointId)}/models/refresh`,
    {},
  );
  return response;
};

const verifyProviderForWorkspace = async (workspaceId, providerId) => {
  const response = await daemonJson("POST", `/api/workspaces/${workspaceId}/providers/${providerId}/verify`, {});
  if (response.status !== 200) {
    throw new Error(`provider verify request failed (${response.status}): ${normalizeErrorMessage(JSON.stringify(response.payload))}`);
  }
  const payload = asRecord(response.payload);
  const status = firstText(payload.status).toLowerCase();
  if (status !== "ok") {
    const detail = normalizeErrorMessage(firstText(payload.message, JSON.stringify(payload)));
    throw new Error(`provider verify failed (status=${status}): ${detail}`);
  }
  return payload;
};

const resolveWorkspaceProviderModelId = async (
  workspaceId,
  providerId,
  { timeoutMs = 60_000, pollMs = 2000 } = {},
) => {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const response = await daemonJson("GET", `/api/workspaces/${workspaceId}/providers/${providerId}/options`);
    if (response.status === 200) {
      const payload = asRecord(response.payload);
      const models = asRecord(payload.models);
      const currentModel = firstText(models.current_model_id, models.currentModelId);
      if (currentModel) return currentModel;
      const firstModel = asArray(models.models)
        .map((entry) => asRecord(entry))
        .map((entry) => firstText(entry.id, entry.model_id, entry.modelId, entry.name))
        .find(Boolean) || "";
      if (firstModel) return firstModel;
    }
    await browser.pause(pollMs);
  }
  throw new Error(`models list not populated for ${providerId} in workspace ${workspaceId}`);
};

const readOpenRouterEnv = () => {
  const apiKey = String(process.env.OPENROUTER_API_KEY || "").trim();
  const baseUrl = String(process.env.OPENROUTER_BASE_URL || "").trim() || DEFAULT_OPENROUTER_BASE_URL;
  const modelOverride = String(process.env.CTX_E2E_OPENROUTER_MODEL_OVERRIDE || "").trim()
    || DEFAULT_CODEX_OPENROUTER_MODEL_OVERRIDE;
  return { apiKey, baseUrl, modelOverride };
};

const ensureCodexOpenRouterWorkspaceReady = async (
  workspaceId,
  {
    installTarget = "host",
    providerId = "codex",
    endpointName = "",
    timeoutMs = 90_000,
    pollMs = 3_000,
  } = {},
) => {
  const { apiKey, baseUrl, modelOverride } = readOpenRouterEnv();
  if (!apiKey) {
    throw new Error("OPENROUTER_API_KEY is required to configure Codex endpoint auth for remote first-turn validation");
  }

  const status = await getProviderStatus(providerId, installTarget);
  if (!status.installed) {
    await installProviderAndWait(providerId, installTarget);
  }

  const endpointId = await configureOpenRouterEndpoint({
    providerId,
    baseUrl,
    apiKey,
    modelOverride,
    endpointName,
  });
  const verifyPayload = await verifyProviderForWorkspace(workspaceId, providerId);
  const modelId = await resolveWorkspaceProviderModelId(workspaceId, providerId, {
    timeoutMs,
    pollMs,
  });

  return {
    providerId,
    endpointId,
    modelId,
    baseUrl,
    modelOverride,
    verifyPayload,
  };
};

module.exports = {
  asRecord,
  asArray,
  readString,
  firstText,
  normalizeErrorMessage,
  getProviderStatus,
  installProviderAndWait,
  configureOpenRouterEndpoint,
  selectHarnessSource,
  selectSubscriptionSource,
  refreshEndpointModels,
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
  readOpenRouterEnv,
  ensureCodexOpenRouterWorkspaceReady,
};
