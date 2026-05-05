const fs = require("node:fs");
const path = require("node:path");

const {
  daemonJson,
  getDesktopConnection = async () => null,
  safeDaemonJson = async (method, requestPath, body) => {
    try {
      return await daemonJson(method, requestPath, body);
    } catch (error) {
      return { error: String(error) };
    }
  },
} = require("./daemon.cjs");
const { providerStatusPath } = require("../../../../../test-support/provider_status_path.cjs");
const { parseBoolish, stringMapFlag } = require("../../../../../scripts/lib/boolish.cjs");

const PROVIDER_MATRIX_PATH = path.resolve(
  __dirname,
  "../../../../../crates/ctx-provider-accounts/src/provider_matrix.json",
);
const DEFAULT_OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_OPENAI_OPENROUTER_MODEL_OVERRIDE = "google/gemini-2.5-flash";
const DEFAULT_CLAUDE_OPENROUTER_MODEL_OVERRIDE = "anthropic/claude-3.5-haiku";
const DEFAULT_QWEN_OPENROUTER_MODEL_OVERRIDE = "google/gemini-2.5-flash";
const DEFAULT_GEMINI_OPENROUTER_MODEL_OVERRIDE = "google/gemini-3-flash-preview";
const ACP_BRIDGE_PROVIDER_ID = "acp-crp-bridge";

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

const splitIdList = (value) =>
  readString(value)
    .split(/[,\s]+/)
    .map((entry) => entry.trim())
    .filter(Boolean);

const providerInstallProgressSnapshot = (status) => {
  const details = readStringMap(status?.details);
  return {
    installed: status?.installed === true,
    health: firstText(status?.health, "unknown"),
    diagnostics: asArray(status?.diagnostics).map((entry) => readString(entry)).filter(Boolean),
    details,
    installRunning: stringMapFlag(details, "install_running"),
    installId: readString(details.install_id),
    installSupported: parseBoolish(details.install_supported),
    detectedPath: readString(status?.detected_path),
    requiredDependencyIds: splitIdList(details.required_dependency_ids),
    pendingDependencyIds: splitIdList(details.pending_dependency_ids),
    readyForUse: parseBoolish(details.ready_for_use),
  };
};

const providerInstallHasProgress = (snapshot) =>
  snapshot.installRunning
  || Boolean(snapshot.installId)
  || snapshot.pendingDependencyIds.length > 0
  || (snapshot.installed && snapshot.readyForUse === false);

const providerInstallComplete = (snapshot) =>
  snapshot.installed
  && !snapshot.installRunning
  && snapshot.pendingDependencyIds.length === 0
  && snapshot.readyForUse !== false;

const providerInstallDiagnosticPayload = (providerId, target, snapshot, extra = {}) => ({
  provider_id: providerId,
  target,
  installed: snapshot.installed,
  health: snapshot.health,
  diagnostics: snapshot.diagnostics,
  detected_path: snapshot.detectedPath,
  install_running: snapshot.installRunning,
  install_id: snapshot.installId,
  install_supported: snapshot.installSupported,
  ready_for_use: snapshot.readyForUse,
  required_dependency_ids: snapshot.requiredDependencyIds,
  pending_dependency_ids: snapshot.pendingDependencyIds,
  details: snapshot.details,
  ...extra,
});

const providerInstallDiagnosticDetail = (providerId, target, snapshot, extra = {}) =>
  JSON.stringify(providerInstallDiagnosticPayload(providerId, target, snapshot, extra));

const pause = async (ms) => {
  if (globalThis.browser && typeof globalThis.browser.pause === "function") {
    await globalThis.browser.pause(ms);
    return;
  }
  await new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));
};

const daemonObservationFromHealth = (health) => {
  const payload = asRecord(health?.payload);
  const compatibility = asRecord(payload.compatibility);
  return {
    status: health?.status ?? null,
    error: readString(health?.error),
    pid: payload.pid ?? null,
    dataRoot: readString(payload.data_root),
    daemonUrl: readString(payload.daemon_url),
    version: firstText(payload.daemon_version, payload.version),
    buildId: readString(compatibility.desktop_build_id),
  };
};

const readDaemonInstallObservation = async () => {
  let connection = null;
  try {
    connection = await getDesktopConnection();
  } catch (error) {
    connection = { error: String(error) };
  }
  const health = await safeDaemonJson("GET", "/api/health");
  const healthObservation = daemonObservationFromHealth(health);
  return {
    connection_kind: readString(connection?.kind),
    base_url: readString(connection?.base_url),
    connection_error: readString(connection?.error),
    ...healthObservation,
  };
};

const daemonInstallObservationChange = (before, after) => {
  if (!before || !after) return null;
  for (const field of ["pid", "dataRoot", "daemonUrl", "version", "buildId"]) {
    if (before[field] === null || after[field] === null) continue;
    if (!before[field] || !after[field]) continue;
    if (before[field] !== after[field]) {
      return {
        field,
        before: before[field],
        after: after[field],
      };
    }
  }
  return null;
};

const parseProviderIdsCsv = (value) =>
  String(value || "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);

const managedProviderIdsFromMatrix = () => {
  const matrix = JSON.parse(fs.readFileSync(PROVIDER_MATRIX_PATH, "utf8"));
  return asArray(matrix.providers)
    .map((entry) => asRecord(entry))
    .filter((entry) => asRecord(entry.managed_install).kind)
    .map((entry) => readString(entry.id).trim())
    .filter(Boolean)
    .sort();
};

const resolveManagedProviderInstallIds = ({ env = process.env } = {}) => {
  const explicit = parseProviderIdsCsv(env.CTX_REMOTE_WORKSPACE_E2E_PROVIDER_IDS);
  if (explicit.length > 0) return explicit;
  return managedProviderIdsFromMatrix();
};

const getProviderStatus = async (providerId, target = "host") => {
  const response = await daemonJson("GET", providerStatusPath(providerId, target));
  if (response.status === 404) {
    return {
      installed: false,
      health: "unknown",
      diagnostics: [`provider not found: ${providerId}`],
      details: {},
    };
  }
  if (response.status !== 200) {
    throw new Error(`failed to read provider '${providerId}' (${response.status})`);
  }
  const row = asRecord(response.payload);
  return {
    installed: row.installed === true,
    detected_path: firstText(row.detected_path),
    version: firstText(row.version),
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
    await pause(pollMs);
  }
  throw new Error(`provider install timed out for ${providerId} (${installId})`);
};

const pollInstallStatusIfPresent = async (installId) => {
  if (!installId) return null;
  const poll = await daemonJson("GET", `/api/providers/install/${installId}`);
  if (poll.status !== 200) {
    return {
      status: poll.status,
      payload: poll.payload,
      state: "",
    };
  }
  const payload = asRecord(poll.payload);
  return {
    status: poll.status,
    payload,
    state: firstText(payload.state).toLowerCase(),
  };
};

const waitForProviderInstallCompletion = async (
  providerId,
  target,
  { timeoutMs = 10 * 60_000, pollMs = 2000, settleMs = 5_000 } = {},
) => {
  const startedAt = Date.now();
  let lastStatus = null;
  const initialDaemonObservation = await readDaemonInstallObservation();
  while (Date.now() - startedAt < timeoutMs) {
    lastStatus = await getProviderStatus(providerId, target);
    const snapshot = providerInstallProgressSnapshot(lastStatus);
    if (providerInstallComplete(snapshot)) return lastStatus;
    if (snapshot.installSupported === false) {
      throw new Error(
        `provider '${providerId}' install is unsupported for target=${target}: ${
          providerInstallDiagnosticDetail(providerId, target, snapshot)
        }`,
      );
    }
    const installInfo = await pollInstallStatusIfPresent(snapshot.installId);
    if (installInfo && (installInfo.state === "failed" || installInfo.state === "cancelled")) {
      throw new Error(
        `provider '${providerId}' install ${installInfo.state} for target=${target}: ${
          providerInstallDiagnosticDetail(providerId, target, snapshot, { install_info: installInfo })
        }`,
      );
    }
    const daemonObservation = await readDaemonInstallObservation();
    const daemonChange = daemonInstallObservationChange(initialDaemonObservation, daemonObservation);
    if (daemonChange) {
      throw new Error(
        `daemon identity changed while waiting for provider '${providerId}' install target=${target}: ${
          providerInstallDiagnosticDetail(providerId, target, snapshot, {
            daemon_initial: initialDaemonObservation,
            daemon_current: daemonObservation,
            daemon_change: daemonChange,
          })
        }`,
      );
    }
    if (!providerInstallHasProgress(snapshot) && Date.now() - startedAt >= settleMs) {
      throw new Error(
        `provider '${providerId}' did not show install progress for target=${target}: ${
          providerInstallDiagnosticDetail(providerId, target, snapshot, {
            daemon_initial: initialDaemonObservation,
            daemon_current: daemonObservation,
          })
        }`,
      );
    }
    await pause(pollMs);
  }
  const snapshot = providerInstallProgressSnapshot(lastStatus);
  throw new Error(
    `provider '${providerId}' install did not finish for target=${target}: ${
      providerInstallDiagnosticDetail(providerId, target, snapshot, {
        daemon_initial: initialDaemonObservation,
        daemon_current: await readDaemonInstallObservation(),
      })
    }`,
  );
};

const installManagedProvidersAndAssertInstalled = async (
  target,
  {
    providerIds = resolveManagedProviderInstallIds(),
    timeoutMs = 10 * 60_000,
    pollMs = 2000,
    recorder = null,
    artifactPrefix = "",
  } = {},
) => {
  const results = [];
  for (const providerId of providerIds) {
    const before = await getProviderStatus(providerId, target);
    const beforeSnapshot = providerInstallProgressSnapshot(before);
    if (!providerInstallComplete(beforeSnapshot) && !before.installed && !beforeSnapshot.installRunning) {
      await installProviderAndWait(providerId, target, { timeoutMs, pollMs });
    }
    if (!providerInstallComplete(beforeSnapshot)) {
      await waitForProviderInstallCompletion(providerId, target, { timeoutMs, pollMs });
    }
    const after = await getProviderStatus(providerId, target);
    const afterSnapshot = providerInstallProgressSnapshot(after);
    const result = {
      provider_id: providerId,
      target,
      before,
      after,
    };
    results.push(result);
    recorder?.recordArtifact?.(
      `${artifactPrefix || target}_managed_provider_${providerId}`,
      result,
    );
    if (!after.installed) {
      throw new Error(`${target}: managed provider '${providerId}' is not installed after install flow`);
    }
    if (!providerInstallComplete(afterSnapshot)) {
      throw new Error(
        `${target}: managed provider '${providerId}' is not ready after install flow: ${
          providerInstallDiagnosticDetail(providerId, target, afterSnapshot)
        }`,
      );
    }
  }
  recorder?.recordAssertion?.(
    `${artifactPrefix || target}_managed_provider_installs`,
    "pass",
    `installed ${providerIds.length} managed providers for target=${target}`,
  );
  return results;
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

const providerDefaultOpenRouterModelOverride = (providerId = "codex") => {
  const normalizedProviderId = firstText(providerId).toLowerCase();
  if (normalizedProviderId === "qwen") {
    return DEFAULT_QWEN_OPENROUTER_MODEL_OVERRIDE;
  }
  if (normalizedProviderId === "claude-crp") {
    return DEFAULT_CLAUDE_OPENROUTER_MODEL_OVERRIDE;
  }
  if (normalizedProviderId === "pi") {
    return DEFAULT_GEMINI_OPENROUTER_MODEL_OVERRIDE;
  }
  return DEFAULT_OPENAI_OPENROUTER_MODEL_OVERRIDE;
};

const readOpenRouterEnv = (providerId = "codex") => {
  const apiKey = String(process.env.OPENROUTER_API_KEY || "").trim();
  const baseUrl = String(process.env.OPENROUTER_BASE_URL || "").trim() || DEFAULT_OPENROUTER_BASE_URL;
  const modelOverride = String(process.env.CTX_E2E_OPENROUTER_MODEL_OVERRIDE || "").trim()
    || providerDefaultOpenRouterModelOverride(providerId);
  return { apiKey, baseUrl, modelOverride };
};

const ensureCodexOpenRouterWorkspaceReady = async (
  workspaceId,
  {
    installTarget = "host",
    providerId = "codex",
    endpointName = "",
    timeoutMs = 90_000,
    installTimeoutMs = 10 * 60_000,
    pollMs = 3_000,
    allowInstall = true,
  } = {},
) => {
  const { apiKey, baseUrl, modelOverride } = readOpenRouterEnv(providerId);
  if (!apiKey) {
    throw new Error("OPENROUTER_API_KEY is required to configure Codex endpoint auth for remote first-turn validation");
  }

  const status = await getProviderStatus(providerId, installTarget);
  const snapshot = providerInstallProgressSnapshot(status);
  if (!providerInstallComplete(snapshot)) {
    if (!allowInstall) {
      throw new Error(
        `provider '${providerId}' is not ready for target=${installTarget}: ${
          providerInstallDiagnosticDetail(providerId, installTarget, snapshot)
        }`,
      );
    }
    if (!snapshot.installed && !snapshot.installRunning) {
      await installProviderAndWait(providerId, installTarget, { timeoutMs: installTimeoutMs, pollMs });
    }
    await waitForProviderInstallCompletion(providerId, installTarget, {
      timeoutMs: installTimeoutMs,
      pollMs,
    });
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

const assertAcpBridgeRuntimeViable = async (target = "host") => {
  const status = await getProviderStatus(ACP_BRIDGE_PROVIDER_ID, target);
  if (!status.installed || status.health !== "ok") {
    const detail = status.diagnostics[0]
      || `installed=${String(status.installed)} health=${status.health || "unknown"}`;
    throw new Error(`ACP bridge runtime is not viable for target=${target}: ${detail}`);
  }
  return status;
};

module.exports = {
  asRecord,
  asArray,
  readString,
  firstText,
  normalizeErrorMessage,
  providerInstallProgressSnapshot,
  providerInstallHasProgress,
  providerInstallDiagnosticPayload,
  daemonInstallObservationChange,
  getProviderStatus,
  installProviderAndWait,
  waitForProviderInstallCompletion,
  resolveManagedProviderInstallIds,
  installManagedProvidersAndAssertInstalled,
  configureOpenRouterEndpoint,
  selectHarnessSource,
  selectSubscriptionSource,
  refreshEndpointModels,
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
  providerDefaultOpenRouterModelOverride,
  readOpenRouterEnv,
  ensureCodexOpenRouterWorkspaceReady,
  assertAcpBridgeRuntimeViable,
};
