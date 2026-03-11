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
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
} = require("./helpers/provider_runtime.cjs");
const {
  createProviderAuthContractRecorder,
  normalizeText,
} = require("./helpers/provider_auth_contract.cjs");
const {
  stageProviderAuthImportFixture,
  listProviderAuthImportCandidates,
  importProviderAuthCandidates,
  listProviderAuthImportProfiles,
  findStagedCandidate,
  assertImportedProfileMetadata,
  assertImportedProfileActive,
} = require("./helpers/provider_auth_import_runtime.cjs");

const parsePositiveInt = (raw, fallback) => {
  const parsed = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
};

const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const createWorkspaceAndLaunchExecution = async ({
  dest,
  name,
  daemonLocation,
  executionEnvironment,
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
  if (executionEnvironment !== "host" && executionEnvironment !== "container_host_mounted") {
    throw new Error(`unsupported execution environment for this spec: ${executionEnvironment}`);
  }

  const environment = executionEnvironment;
  const networkMode = executionEnvironment === "container_host_mounted" ? "llm_only" : "all";

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

describe("provider auth import matrix cell (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const providerId = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_PROVIDER_ID || "codex");
  const authMode = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_AUTH_MODE || "auth_import");
  const daemonLocation = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_DAEMON_LOCATION || "local");
  const executionEnvironment = normalizeText(
    process.env.CTX_PROVIDER_AUTH_MATRIX_EXECUTION_ENVIRONMENT || "container_host_mounted",
  );
  const cellId = normalizeText(
    process.env.CTX_PROVIDER_AUTH_MATRIX_CELL_ID
      || `${providerId}.${authMode}.${daemonLocation}.${executionEnvironment}`,
  );
  const reportPath = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_REPORT)
    || path.join("/tmp", `ctx-provider-auth-import-${cellId.replace(/[^a-zA-Z0-9._-]+/g, "_")}-${runId}.json`);
  const localBase = mkTempDir(`ctx-provider-auth-import-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?providerAuthImportMatrixCell=${Date.now()}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it(`executes provider auth import contract for ${cellId}`, async function () {
    this.timeout(parsePositiveInt(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "1200000", 1200000));
    if (!scenarioEnabled("provider-auth-import", ["provider", "auth", "import", providerId])) this.skip();
    if (authMode !== "auth_import") {
      throw new Error(`auth import spec requires auth_mode=auth_import (got ${authMode})`);
    }

    const recorder = createProviderAuthContractRecorder({
      outputPath: reportPath,
      cell: cellId,
      providerId,
      authMode,
      daemonLocation,
      executionEnvironment,
    });

    let currentAssertion = "auth_import_success";
    let workspaceLaunch = null;

    try {
      const stagedFixture = stageProviderAuthImportFixture(providerId);
      recorder.recordArtifact("staged_fixture", stagedFixture);

      await assertConnectedLocalAndListening();
      recorder.recordArtifact("desktop_connection", { ok: true });

      currentAssertion = "candidate_detected";
      const candidates = await listProviderAuthImportCandidates();
      recorder.recordArtifact("auth_import_candidates", candidates);
      const candidate = findStagedCandidate(candidates, stagedFixture);
      recorder.recordArtifact("selected_candidate", candidate);
      if (normalizeText(candidate.parse_status).toLowerCase() !== "parsed") {
        throw new Error(`selected candidate is not importable: ${JSON.stringify(candidate)}`);
      }

      const workspaceDest = path.join(localBase, cellId.replace(/\./g, "-"));
      const workspace = await createWorkspaceAndLaunchExecution({
        dest: workspaceDest,
        name: `provider-auth-import-${providerId}-${Date.now()}`,
        daemonLocation,
        executionEnvironment,
        onLaunchStart: async (launchInfo) => {
          workspaceLaunch = launchInfo;
          recorder.recordArtifact("workspace_launch_started", launchInfo);
        },
      });
      recorder.recordArtifact("workspace", workspace);
      recorder.recordAssertion("candidate_detected", "pass", "staged auth import candidate was detected");

      if (executionEnvironment === "container_host_mounted") {
        await assertLocalWorkspaceConfig(workspace.workspaceId, {
          environment: "container_host_mounted",
          networkMode: "llm_only",
        });
      }

      currentAssertion = "install_success";
      const installTarget = executionEnvironment === "container_host_mounted" ? "container" : "host";
      await installProviderAndWait(providerId, installTarget);
      const providerStatus = await getProviderStatus(providerId, installTarget);
      recorder.recordArtifact("provider_status_after_install", providerStatus);
      if (!providerStatus.installed) {
        throw new Error(`provider not installed after install flow: ${JSON.stringify(providerStatus)}`);
      }
      recorder.recordAssertion("install_success", "pass", "provider install completed");

      currentAssertion = "auth_import_success";
      const importResults = await importProviderAuthCandidates([candidate.id]);
      recorder.recordArtifact("auth_import_results", importResults);
      const result = importResults.find((entry) => normalizeText(entry.candidate_id) === normalizeText(candidate.id));
      if (!result) {
        throw new Error(`auth import response missing result for candidate ${candidate.id}`);
      }
      const acceptableStatuses = new Set(["imported", "updated", "already_imported"]);
      if (!acceptableStatuses.has(normalizeText(result.status))) {
        throw new Error(`auth import failed: ${JSON.stringify(result)}`);
      }
      const profiles = await listProviderAuthImportProfiles();
      recorder.recordArtifact("auth_import_profiles", profiles);
      const importedProfile = assertImportedProfileMetadata({ profiles, candidate, result });
      recorder.recordArtifact("imported_profile_metadata", importedProfile);
      const activeProfile = await assertImportedProfileActive({ providerId, candidate, result });
      recorder.recordArtifact("active_profile_state", activeProfile);
      recorder.recordAssertion(
        "auth_import_success",
        "pass",
        `import produced active profile '${normalizeText(result.profile_id)}'`,
      );

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
      recorder.recordArtifact("provider_options_after_import", optionsResp.payload || null);
      recorder.recordArtifact("resolved_model_id", modelId);
      recorder.recordAssertion("model_list_population", "pass", `resolved model id '${modelId}'`);

      currentAssertion = "first_turn_success";
      const turnResult = await runProviderFirstTurnApiSmoke(
        workspace.workspaceId,
        {
          providerId,
          modelId,
          executionEnvironment,
          prompt: `provider-auth-import-${cellId}-${Date.now()}: reply with exactly pong`,
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
