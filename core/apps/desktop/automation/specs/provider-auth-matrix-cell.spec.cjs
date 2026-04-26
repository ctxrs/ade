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
  getWorkspaceTerminalCwd,
  runProviderFirstTurnApiSmoke,
  runProviderFirstTurnApiExpectedFailure,
  runProviderFileEditApiSmoke,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  getProviderStatus,
  installProviderAndWait,
  configureOpenRouterEndpoint,
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
  readOpenRouterEnv,
} = require("./helpers/provider_runtime.cjs");
const {
  createProviderAuthContractRecorder,
  normalizeText,
} = require("./helpers/provider_auth_contract.cjs");
const {
  providerAuthMatrixScenarioTags,
} = require("./helpers/provider_auth_matrix_scenarios.cjs");
const {
  prepareSubscriptionAuth,
} = require("./helpers/provider_auth_matrix_flow.cjs");

const DEFAULT_PROVIDER_ID = "codex";
const DEFAULT_AUTH_MODE = "endpoint_api_key";
const DEFAULT_DAEMON_LOCATION = "local";
const DEFAULT_EXECUTION_ENVIRONMENT = "sandbox";

const parsePositiveInt = (raw, fallback) => {
  const parsed = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
};

const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));
const BLOCKED_NETWORK_ALLOWLIST = ["github.com"];
const shouldAssertSandboxNetworkBlockFailure = ({
  providerId,
  authMode,
  daemonLocation,
  executionEnvironment,
}) => (
  providerId === "codex"
  && daemonLocation === "local"
  && executionEnvironment === "sandbox"
  && (authMode === "endpoint_api_key" || authMode === "configure_later_then_connect")
);

const createWorkspaceAndLaunchExecution = async ({
  dest,
  name,
  daemonLocation,
  executionEnvironment,
  networkMode = "",
  allowlist = [],
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

  if (daemonLocation === "remote") {
    throw new Error(`remote daemon location is not supported by this spec: ${daemonLocation}`);
  }
  if (executionEnvironment !== "host" && executionEnvironment !== "sandbox") {
    throw new Error(`unsupported execution environment for this spec: ${executionEnvironment}`);
  }

  const environment = executionEnvironment;
  const resolvedNetworkMode = normalizeText(networkMode) || (executionEnvironment === "sandbox" ? "llm_only" : "all");
  const setExecutionConfigPayload = {
    environment,
    network_mode: resolvedNetworkMode,
  };
  if (resolvedNetworkMode === "allowlist") {
    setExecutionConfigPayload.allowlist = Array.isArray(allowlist) ? allowlist : [];
  }

  const setExec = await daemonJson(
    "POST",
    `/api/workspaces/${workspaceId}/execution_config`,
    setExecutionConfigPayload,
  );
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
      networkMode: resolvedNetworkMode,
      allowlist: resolvedNetworkMode === "allowlist" ? setExecutionConfigPayload.allowlist : [],
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
        const executionRoot = normalizeText(await getWorkspaceTerminalCwd(workspaceId));
        if (!executionRoot) {
          throw new Error(`workspace execution root missing for ${workspaceId}`);
        }
        return {
          workspaceId,
          environment,
          networkMode: resolvedNetworkMode,
          allowlist: resolvedNetworkMode === "allowlist" ? setExecutionConfigPayload.allowlist : [],
          executionRoot,
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

const openRouterEnv = (providerId) => readOpenRouterEnv(providerId);

describe("provider auth matrix cell (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const providerId = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_PROVIDER_ID || DEFAULT_PROVIDER_ID);
  const authMode = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_AUTH_MODE || DEFAULT_AUTH_MODE);
  const daemonLocation = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_DAEMON_LOCATION || DEFAULT_DAEMON_LOCATION);
  const executionEnvironment = normalizeText(
    process.env.CTX_PROVIDER_AUTH_MATRIX_EXECUTION_ENVIRONMENT || DEFAULT_EXECUTION_ENVIRONMENT,
  );
  const cellId = normalizeText(
    process.env.CTX_PROVIDER_AUTH_MATRIX_CELL_ID
      || `${providerId}.${authMode}.${daemonLocation}.${executionEnvironment}`,
  );
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
    if (!scenarioEnabled(
      "provider-auth-matrix",
      providerAuthMatrixScenarioTags({
        cellId,
        providerId,
        authMode,
        daemonLocation,
        executionEnvironment,
      }),
    )) this.skip();

    const recorder = createProviderAuthContractRecorder({
      outputPath: reportPath,
      cell: cellId,
      providerId,
      authMode,
      daemonLocation,
      executionEnvironment,
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
        daemonLocation,
        executionEnvironment,
        onLaunchStart: async (launchInfo) => {
          workspaceLaunch = launchInfo;
          recorder.recordArtifact("workspace_launch_started", launchInfo);
        },
      });
      recorder.recordArtifact("workspace", workspace);
      recorder.recordAssertion("workspace_launch_success", "pass", "workspace launch reached ready state");

      if (executionEnvironment === "sandbox") {
        await assertLocalWorkspaceConfig(workspace.workspaceId, {
          environment: "sandbox",
          networkMode: "llm_only",
        });
      }

      currentAssertion = "install_success";
      const installTarget = executionEnvironment === "sandbox" ? "container" : "host";
      await installProviderAndWait(providerId, installTarget);
      const providerStatus = await getProviderStatus(providerId, installTarget);
      recorder.recordArtifact("provider_status_after_install", providerStatus);
      if (!providerStatus.installed) {
        throw new Error(`provider not installed after install flow: ${JSON.stringify(providerStatus)}`);
      }
      recorder.recordAssertion("install_success", "pass", "provider install completed");

      if (authMode === "subscription_oauth") {
        currentAssertion = "subscription_auth_setup";
        const authSetup = await prepareSubscriptionAuth({
          providerId,
          daemonLocation,
          executionEnvironment,
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
        if (authSetup.artifacts?.oauth_login?.status === "opened") {
          recorder.recordAssertion(
            "subscription_browser_open_success",
            "pass",
            "subscription auth flow opened the provider auth endpoint through the desktop shell path",
          );
        }
        recorder.recordAssertion(
          "subscription_auth_setup",
          "pass",
          "managed subscription auth prepared for verify and first-turn validation",
        );
      } else if (authMode === "endpoint_api_key" || authMode === "configure_later_then_connect") {
        const { apiKey, baseUrl, modelOverride } = openRouterEnv(providerId);
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
          executionEnvironment,
          prompt: `provider-auth-matrix-${Date.now()}: reply with exactly pong`,
        },
        240_000,
      );
      recorder.recordArtifact("first_turn_result", turnResult);
      recorder.recordAssertion("first_turn_success", "pass", "first turn completed with assistant response");

      if (shouldAssertSandboxNetworkBlockFailure({
        providerId,
        authMode,
        daemonLocation,
        executionEnvironment,
      })) {
        currentAssertion = "network_block_failure";
        const blockedWorkspaceDest = path.join(localBase, `${cellId.replace(/\./g, "-")}-blocked-network`);
        const blockedWorkspace = await createWorkspaceAndLaunchExecution({
          dest: blockedWorkspaceDest,
          name: `provider-auth-blocked-${providerId}-${Date.now()}`,
          daemonLocation,
          executionEnvironment: "sandbox",
          networkMode: "allowlist",
          allowlist: BLOCKED_NETWORK_ALLOWLIST,
        });
        recorder.recordArtifact("blocked_network_workspace", blockedWorkspace);
        await assertLocalWorkspaceConfig(blockedWorkspace.workspaceId, {
          environment: "sandbox",
          networkMode: "allowlist",
          allowlist: BLOCKED_NETWORK_ALLOWLIST,
        });
        const blockedTurnResult = await runProviderFirstTurnApiExpectedFailure(
          blockedWorkspace.workspaceId,
          {
            providerId,
            modelId,
            executionEnvironment: "sandbox",
            prompt: `provider-auth-matrix-${Date.now()}: reply with exactly pong`,
            requiredErrorSubstrings: ["blocked by allowlist"],
            forbiddenErrorSubstrings: ["provider_startup_timeout", "did not emit any events within"],
          },
          240_000,
        );
        recorder.recordArtifact("blocked_network_first_turn_result", blockedTurnResult);
        recorder.recordAssertion(
          "network_block_failure",
          "pass",
          `blocked sandbox turn failed fast with allowlist error: ${blockedTurnResult.errorMessage}`,
        );
      }

      currentAssertion = "file_edit_success";
      const fileEditResult = await runProviderFileEditApiSmoke(
        workspace.workspaceId,
        workspace.executionRoot,
        {
          providerId,
          modelId,
          executionEnvironment,
          relativeFilePath: "hello.md",
          fileContents: "hi",
          exactFileContents: true,
          expectedAssistantMessage: "hi",
          exactAssistantMessage: true,
          prompt: [
            "This is an end to end test, so it is very important that you do exactly what I ask.",
            "Make a new file in the workspace root called hello.md and put exactly this text in it: hi. The file must contain exactly those two characters with no trailing newline or extra whitespace. If you use a shell command to write the file, use printf rather than echo -n, because echo -n is not portable and may write the literal text -n.",
            "Use only the current worktree root as the target directory. Do not write in a parent directory, and if your first attempt adds a trailing newline or uses the wrong directory, fix the file before replying.",
            "That is all. Do it now without further deliberation.",
            "After writing the file, reply with exactly: hi",
          ].join(" "),
        },
        240_000,
      );
      recorder.recordArtifact("file_edit_result", fileEditResult);
      recorder.recordAssertion("file_edit_success", "pass", "provider created hello.md with exact contents");

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
