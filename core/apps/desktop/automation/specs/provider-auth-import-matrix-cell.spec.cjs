const fs = require("node:fs");
const path = require("node:path");

const { navigateToTauriUrl } = require("./helpers/tauri.cjs");
const { daemonJson, safeDaemonJson, checkDaemonHealth } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  initGitRepo,
  scenarioEnabled,
  assertConnectedLocalAndListening,
  assertLocalWorkspaceConfig,
  getWorkspaceTerminalCwd,
  runProviderFirstTurnApiSmoke,
  runProviderFileEditApiSmoke,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  createProviderAuthImportWorkspaceAndLaunchExecution,
} = require("./helpers/provider_auth_import_workspace_launch.cjs");
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

const ensureLocalLinuxSandboxReady = async () => {
  const result = await browser.executeAsync(({ req }, done) => {
    const tauriCoreInvoke = window.__TAURI__?.core?.invoke;
    const tauriInternalsInvoke = window.__TAURI_INTERNALS__?.invoke;
    const invoke = tauriInternalsInvoke || tauriCoreInvoke;
    if (!invoke) {
      done({ error: "Tauri invoke API not available" });
      return;
    }
    Promise.resolve()
      .then(() => invoke("desktop_ensure_local_linux_sandbox_ready", { req }))
      .then((value) => done({ value }))
      .catch((error) => done({ error: String(error) }));
  }, { req: { admin_password_once: null } });
  if (result && typeof result === "object" && result.error) {
    throw new Error(`local Linux sandbox ensure failed: ${result.error}`);
  }
  if (!result || typeof result !== "object" || !result.value || result.value.ready !== true) {
    throw new Error(`local Linux sandbox ensure returned unexpected payload: ${JSON.stringify(result || null)}`);
  }
  return result.value;
};

describe("provider auth import matrix cell (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const providerId = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_PROVIDER_ID || "codex");
  const authMode = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_AUTH_MODE || "auth_import");
  const daemonLocation = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_DAEMON_LOCATION || "local");
  const executionEnvironment = normalizeText(
    process.env.CTX_PROVIDER_AUTH_MATRIX_EXECUTION_ENVIRONMENT || "sandbox",
  );
  const cellId = normalizeText(
    process.env.CTX_PROVIDER_AUTH_MATRIX_CELL_ID
      || `${providerId}.${authMode}.${daemonLocation}.${executionEnvironment}`,
  );
  const reportPath = normalizeText(process.env.CTX_PROVIDER_AUTH_MATRIX_REPORT)
    || path.join("/tmp", `ctx-provider-auth-import-${cellId.replace(/[^a-zA-Z0-9._-]+/g, "_")}-${runId}.json`);
  const localBase = mkTempDir(`ctx-provider-auth-import-${runId}-`);

  before(async () => {
    await navigateToTauriUrl(`tauri://localhost/workspace-setup?providerAuthImportMatrixCell=${Date.now()}`);
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
      const workspace = await createProviderAuthImportWorkspaceAndLaunchExecution({
        dest: workspaceDest,
        name: `provider-auth-import-${providerId}-${Date.now()}`,
        daemonLocation,
        executionEnvironment,
        platform: process.platform,
        onLaunchStart: async (launchInfo) => {
          workspaceLaunch = launchInfo;
          recorder.recordArtifact("workspace_launch_started", launchInfo);
        },
        initGitRepo,
        daemonJson,
        ensureLocalLinuxSandboxReady,
        getWorkspaceTerminalCwd,
      });
      recorder.recordArtifact("workspace", workspace);
      recorder.recordAssertion("candidate_detected", "pass", "staged auth import candidate was detected");

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
