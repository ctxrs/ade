const fs = require("node:fs");
const path = require("node:path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson, safeDaemonJson, checkDaemonHealth } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  initGitRepo,
  scenarioEnabled,
  assertConnectedLocalAndListening,
  assertLocalWorkspaceConfig,
  runProviderFirstTurnApiSmoke,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  getProviderStatus,
  installProviderAndWait,
  configureOpenRouterEndpoint,
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
} = require("./helpers/provider_runtime.cjs");
const {
  createProviderAuthContractRecorder,
  normalizeText,
} = require("./helpers/provider_auth_contract.cjs");
const {
  prepareSubscriptionAuth,
} = require("./helpers/provider_auth_matrix_flow.cjs");

const DEFAULT_OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_MODEL_OVERRIDE = "openai/gpt-5.2-codex";
const DEFAULT_PROVIDER_ID = "codex";
const DEFAULT_AUTH_MODE = "endpoint_api_key";
const DEFAULT_ENV_TARGET = "local_container";

const parsePositiveInt = (raw, fallback) => {
  const parsed = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
};

const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const createWorkspaceAndLaunchExecution = async ({
  dest,
  name,
  envTarget,
  timeoutMs = 15 * 60_000,
  onLaunchStart = null,
}) => {
  initGitRepo(dest, name);
  const create = await daemonJson("POST", "/api/workspaces", {
    root_path: dest,
    name,
  });
  if (create.status !== 200) {
    throw new Error(`workspace create failed (${create.status}): ${JSON.stringify(create.payload || null)}`);
  }
  const workspaceId = normalizeText(create.payload?.id);
  if (!workspaceId) {
    throw new Error(`workspace create response missing id: ${JSON.stringify(create.payload || null)}`);
  }

  if (envTarget === "remote_host" || envTarget === "remote_container") {
    throw new Error(`remote env target is not supported by this spec: ${envTarget}`);
  }

  const environment = envTarget === "local_container" ? "container_host_mounted" : "host";
  const networkMode = envTarget === "local_container" ? "llm_only" : "all";

  const setExec = await daemonJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment,
    network_mode: networkMode,
  });
  if (setExec.status !== 200) {
    throw new Error(`execution config update failed (${setExec.status}): ${JSON.stringify(setExec.payload || null)}`);
  }

  const launch = await daemonJson("POST", "/api/execution/launch/start", {
    workspace_id: workspaceId,
  });
  if (launch.status !== 200) {
    throw new Error(`execution launch start failed (${launch.status}): ${JSON.stringify(launch.payload || null)}`);
  }
  const jobId = normalizeText(launch.payload?.job_id);
  if (!jobId) {
    throw new Error(`execution launch response missing job_id: ${JSON.stringify(launch.payload || null)}`);
  }
  if (typeof onLaunchStart === "function") {
    await onLaunchStart({
      workspaceId,
      environment,
      networkMode,
      launchJobId: jobId,
      launchStart: launch.payload || null,
    });
  }

  const startedAt = Date.now();
  let lastPayload = null;
  while (Date.now() - startedAt < timeoutMs) {
    const status = await daemonJson("GET", `/api/execution/launch/status?job_id=${encodeURIComponent(jobId)}`);
    if (status.status === 200) {
      lastPayload = status.payload || null;
      const state = normalizeText(status.payload?.state).toLowerCase();
      if (state === "ready") {
        return {
          workspaceId,
          environment,
          networkMode,
          launchJobId: jobId,
          launchStatus: status.payload || null,
        };
      }
      if (state === "error") {
        throw new Error(`execution launch failed: ${JSON.stringify(status.payload || null)}`);
      }
    }
    await waitMs(1000);
  }

  throw new Error(
    `execution launch timed out for workspace=${workspaceId} job=${jobId} last=${JSON.stringify(lastPayload)}`,
  );
};

const openRouterEnv = () => {
  const apiKey = normalizeText(process.env.OPENROUTER_API_KEY || "");
  const baseUrl = normalizeText(process.env.OPENROUTER_BASE_URL || "") || DEFAULT_OPENROUTER_BASE_URL;
  const modelOverride = normalizeText(process.env.CTX_E2E_OPENROUTER_MODEL_OVERRIDE || "") || DEFAULT_MODEL_OVERRIDE;
  return { apiKey, baseUrl, modelOverride };
};

describe("provider auth matrix cell (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const providerId = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_PROVIDER_ID || DEFAULT_PROVIDER_ID);
  const authMode = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_AUTH_MODE || DEFAULT_AUTH_MODE);
  const envTarget = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_ENV_TARGET || DEFAULT_ENV_TARGET);
  const cellId = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_CELL_ID || `${providerId}.${authMode}.${envTarget}`);
  const reportPath = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_REPORT)
    || path.join("/tmp", `ctx-provider-auth-matrix-${cellId.replace(/[^a-zA-Z0-9._-]+/g, "_")}-${runId}.json`);
  const localBase = mkTempDir(`ctx-provider-auth-matrix-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?providerAuthMatrixCell=${Date.now()}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it(`executes provider auth matrix contract for ${cellId}`, async function () {
    this.timeout(parsePositiveInt(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "1200000", 1200000));
    if (!scenarioEnabled("provider-auth-matrix", ["local", "provider", "matrix", providerId, authMode])) this.skip();

    const recorder = createProviderAuthContractRecorder({
      outputPath: reportPath,
      cell: cellId,
      providerId,
      authMode,
      envTarget,
    });

    let currentAssertion = "workspace_launch_success";
    let workspaceLaunch = null;

    try {
      await assertConnectedLocalAndListening();
      recorder.recordArtifact("desktop_connection", { ok: true });

      const workspaceDest = path.join(localBase, cellId.replace(/\./g, "-"));
      const workspace = await createWorkspaceAndLaunchExecution({
        dest: workspaceDest,
        name: `provider-auth-${providerId}-${Date.now()}`,
        envTarget,
        onLaunchStart: async (launchInfo) => {
          workspaceLaunch = launchInfo;
          recorder.recordArtifact("workspace_launch_started", launchInfo);
        },
      });
      recorder.recordArtifact("workspace", workspace);
      recorder.recordAssertion("workspace_launch_success", "pass", "workspace launch reached ready state");

      if (envTarget === "local_container") {
        await assertLocalWorkspaceConfig(workspace.workspaceId, {
          environment: "container_host_mounted",
          networkMode: "llm_only",
        });
      }

      currentAssertion = "install_success";
      await installProviderAndWait(providerId, envTarget === "local_container" ? "container" : "host");
      const providerStatus = await getProviderStatus(providerId, envTarget === "local_container" ? "container" : "host");
      recorder.recordArtifact("provider_status_after_install", providerStatus);
      if (!providerStatus.installed) {
        throw new Error(`provider not installed after install flow: ${JSON.stringify(providerStatus)}`);
      }
      recorder.recordAssertion("install_success", "pass", "provider install completed");

      if (authMode === "subscription_oauth") {
        currentAssertion = "subscription_auth_setup";
        const authSetup = await prepareSubscriptionAuth({
          providerId,
          envTarget,
        });
        recorder.recordArtifact("subscription_auth_setup", authSetup.artifacts || null);
        if (authSetup.status === "skip") {
          recorder.recordAssertion("subscription_auth_setup", "skip", authSetup.reason || "subscription auth skipped");
          recorder.finalize({
            result: "skip",
            reason: authSetup.reason || "subscription auth skipped",
            extras: { workspace_id: workspace.workspaceId },
          });
          return;
        }
        recorder.recordAssertion(
          "subscription_auth_setup",
          "pass",
          "managed subscription auth prepared for verify and first-turn validation",
        );
      } else if (authMode === "endpoint_api_key" || authMode === "configure_later_then_connect") {
        const { apiKey, baseUrl, modelOverride } = openRouterEnv();
        if (!apiKey) {
          throw new Error("OPENROUTER_API_KEY is required for endpoint/configure-later matrix cells");
        }

        currentAssertion = "configure_later_connect_success";
        if (authMode === "configure_later_then_connect") {
          const preOptions = await daemonJson(
            "GET",
            `/api/workspaces/${workspace.workspaceId}/providers/${providerId}/options`,
          );
          recorder.recordArtifact("provider_options_before_auth", preOptions.payload || null);
        }

        const endpointId = await configureOpenRouterEndpoint({
          providerId,
          baseUrl,
          apiKey,
          modelOverride,
          endpointName: `${providerId}-matrix-${Date.now()}`,
        });
        recorder.recordArtifact("selected_endpoint_id", endpointId);
        if (authMode === "configure_later_then_connect") {
          recorder.recordAssertion(
            "configure_later_connect_success",
            "pass",
            "auth configured after workspace creation and before verify/run",
          );
        }
      } else {
        throw new Error(`auth mode not implemented by this spec: ${authMode}`);
      }

      currentAssertion = "probe_success";
      const verifyPayload = await verifyProviderForWorkspace(workspace.workspaceId, providerId);
      recorder.recordArtifact("verify_response", verifyPayload);
      recorder.recordAssertion("probe_success", "pass", "provider verify returned ok");

      currentAssertion = "model_list_population";
      const modelId = await resolveWorkspaceProviderModelId(workspace.workspaceId, providerId, {
        timeoutMs: 90_000,
        pollMs: 3_000,
      });
      const optionsResp = await daemonJson(
        "GET",
        `/api/workspaces/${workspace.workspaceId}/providers/${providerId}/options`,
      );
      recorder.recordArtifact("provider_options_after_auth", optionsResp.payload || null);
      recorder.recordArtifact("resolved_model_id", modelId);
      recorder.recordAssertion("model_list_population", "pass", `resolved model id '${modelId}'`);

      currentAssertion = "first_turn_success";
      const turnResult = await runProviderFirstTurnApiSmoke(
        workspace.workspaceId,
        {
          providerId,
          modelId,
          prompt: `provider-auth-matrix-${Date.now()}: reply with exactly pong`,
        },
        240_000,
      );
      recorder.recordArtifact("first_turn_result", turnResult);
      recorder.recordAssertion("first_turn_success", "pass", "first turn completed with assistant response");

      recorder.finalize({
        result: "pass",
        reason: "cell passed",
        extras: { workspace_id: workspace.workspaceId },
      });
    } catch (error) {
      if (workspaceLaunch?.launchJobId) {
        const launchStatus = await safeDaemonJson(
          "GET",
          `/api/execution/launch/status?job_id=${encodeURIComponent(workspaceLaunch.launchJobId)}`,
        );
        recorder.recordArtifact("workspace_launch_status_on_failure", launchStatus.payload || launchStatus);
      }
      recorder.recordArtifact("daemon_health_on_failure", await checkDaemonHealth());
      const diagnostics = await safeDaemonJson("GET", "/api/diagnostics");
      recorder.recordArtifact("daemon_diagnostics_on_failure", diagnostics.payload || diagnostics);
      recorder.recordAssertion(currentAssertion, "fail", String(error));
      recorder.finalize({
        result: "fail",
        reason: "cell failed",
        error: String(error),
      });
      throw error;
    }
  });
});
