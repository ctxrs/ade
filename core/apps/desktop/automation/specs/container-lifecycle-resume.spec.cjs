const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { sampleDaemonHealth } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  assertConnectedLocalAndListening,
  assertLocalWorkspaceConfig,
  runCodexFirstTurnApiSmoke,
  getWorkspaceHarnessContainer,
  assertNoDaemonOverlayFor,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  ensureCodexOpenRouterWorkspaceReady,
  readOpenRouterEnv,
} = require("./helpers/provider_runtime.cjs");
const {
  createAndLaunchContainerWorkspace,
  resolveDesktopPodmanControl,
  stopPodmanMachineAndWait,
  startPodmanMachineAndWait,
  restartLocalDaemonAndWait,
  restartDesktopAppAndWait,
  openWorkspaceRouteAndWait,
  collectLifecycleDiagnostics,
  writeLifecycleReport,
} = require("./helpers/container_lifecycle.cjs");

const reportPath = process.env.CTX_CONTAINER_LIFECYCLE_RESUME_REPORT
  || path.join("/tmp", "ctx-container-lifecycle-resume.json");
const scenarioFilter = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean),
);
const lifecycleScenarioTokens = new Set([
  "local-runtime-resume",
  "local-app-daemon-resume",
  "nightly-lifecycle-combined",
  "lifecycle",
  "runtime-resume",
  "app-daemon-resume",
]);
const lifecycleScenarioFilter = new Set(
  [...scenarioFilter].filter((token) => lifecycleScenarioTokens.has(token)),
);
const CASE_TIMEOUT_MS = Number.parseInt(
  String(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "1500000"),
  10,
) || 1500000;
const podmanMachineSupported = process.platform === "darwin" || process.platform === "win32";

const scenarioEnabled = (name, tags = []) => {
  if (lifecycleScenarioFilter.size === 0) return true;
  return [name, ...tags].some((token) => lifecycleScenarioFilter.has(String(token).trim().toLowerCase()));
};

const explicitScenarioEnabled = (name) => scenarioFilter.has(String(name).trim().toLowerCase());

const trimPreview = (value, limit = 220) => {
  const text = String(value || "").trim();
  if (text.length <= limit) return text;
  return `${text.slice(0, limit)}...`;
};

const runId = `${Date.now()}`;
const localBase = mkTempDir(`ctx-container-lifecycle-resume-${runId}-`);
const report = {
  run_id: runId,
  report_path: reportPath,
  cases: [],
  updated_at: new Date().toISOString(),
};

const writeReport = () => {
  report.updated_at = new Date().toISOString();
  writeLifecycleReport(reportPath, report);
};

const upsertCaseReport = (caseReport) => {
  const next = { ...caseReport, updated_at: new Date().toISOString() };
  const existingIndex = report.cases.findIndex((entry) => entry.id === next.id);
  if (existingIndex >= 0) {
    report.cases.splice(existingIndex, 1, next);
  } else {
    report.cases.push(next);
  }
  writeReport();
};

const stageLog = (caseId, label, details = "") => {
  const suffix = details ? ` ${details}` : "";
  // eslint-disable-next-line no-console
  console.log(`[container-lifecycle-resume:${caseId}] ${new Date().toISOString()} ${label}${suffix}`);
};

const markSkipped = function markSkipped(caseReport, reason) {
  caseReport.status = "skipped";
  caseReport.skip_reason = reason;
  upsertCaseReport(caseReport);
  this.skip();
};

const prepareWorkspaceContext = async (caseReport, label) => {
  const dest = path.join(localBase, label);
  stageLog(caseReport.id, "workspace.launch.start", `dest=${dest}`);
  const launch = await createAndLaunchContainerWorkspace({
    dest,
    name: label,
    environment: "sandbox",
    networkMode: "llm_only",
    log: (phase, details) => stageLog(caseReport.id, phase, details),
  });
  caseReport.workspace = {
    id: launch.workspaceId,
    dest,
    launch,
  };
  upsertCaseReport(caseReport);

  await assertConnectedLocalAndListening();
  await assertLocalWorkspaceConfig(launch.workspaceId, {
    environment: "sandbox",
    networkMode: "llm_only",
  });

  stageLog(caseReport.id, "provider.ready.start");
  const provider = await ensureCodexOpenRouterWorkspaceReady(launch.workspaceId, {
    installTarget: "container",
    endpointName: `codex-openrouter-lifecycle-${runId}-${label}`,
    timeoutMs: 90_000,
    pollMs: 3_000,
  });
  caseReport.provider = {
    provider_id: provider.providerId,
    endpoint_id: provider.endpointId,
    model_id: provider.modelId,
    model_override: provider.modelOverride,
    base_url: provider.baseUrl,
  };
  upsertCaseReport(caseReport);

  stageLog(caseReport.id, "workspace.route.open");
    await openWorkspaceRouteAndWait(launch.workspaceId);
    await assertNoDaemonOverlayFor(10_000);
    const container = await getWorkspaceHarnessContainer(launch.workspaceId);
    if (!container || !container.running) {
      throw new Error(`expected running sandbox harness container, got ${JSON.stringify(container)}`);
    }
  caseReport.container = {
    initial: container,
  };
  upsertCaseReport(caseReport);

  return {
    workspaceId: launch.workspaceId,
    modelId: provider.modelId,
  };
};

const runFirstTurnPhase = async (caseReport, context, label) => {
  stageLog(caseReport.id, `${label}.start`);
  const result = await runCodexFirstTurnApiSmoke(
    context.workspaceId,
    {
      providerId: "codex",
      modelId: context.modelId,
      prompt: "hello",
    },
    240_000,
  );
  if (!Array.isArray(caseReport.turns)) {
    caseReport.turns = [];
  }
  caseReport.turns.push({
    label,
    task_id: result.taskId,
    session_id: result.sessionId,
    assistant_preview: trimPreview(result.assistantMessage),
  });
  upsertCaseReport(caseReport);
  return result;
};

const verifyDaemonHealthPhase = async (caseReport, label, durationMs) => {
  stageLog(caseReport.id, `${label}.health.start`, `duration_ms=${durationMs}`);
  const health = await sampleDaemonHealth({ durationMs, intervalMs: 2_000 });
  caseReport[label] = health;
  upsertCaseReport(caseReport);
  if (!health.ok) {
    throw new Error(`daemon health degraded during ${label}: ${JSON.stringify(health.failures)}`);
  }
};

describe("container lifecycle resume (desktop e2e)", () => {
  before(async () => {
    writeReport();
  });

  beforeEach(async () => {
    await browser.url(`tauri://localhost/workspaces?containerLifecycleResume=${Date.now()}`);
    await waitForTauri();
  });

  after(() => {
    writeReport();
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("reopens an existing container workspace after podman machine stop/start and completes the next first turn", async function () {
    const caseReport = {
      id: "local-runtime-resume",
      title: "runtime stop/start -> reopen workspace -> first turn succeeds",
      status: "running",
      started_at: new Date().toISOString(),
    };
    upsertCaseReport(caseReport);
    if (!scenarioEnabled("local-runtime-resume", ["lifecycle", "runtime-resume"])) {
      return markSkipped.call(this, caseReport, "scenario filter excluded runtime resume case");
    }
    if (!podmanMachineSupported) {
      return markSkipped.call(this, caseReport, `podman machine lifecycle not supported on ${process.platform}`);
    }
    const { apiKey } = readOpenRouterEnv();
    if (!apiKey) {
      // eslint-disable-next-line no-console
      console.error("[skip] OPENROUTER_API_KEY is required for container-lifecycle-resume.spec.cjs runtime-resume");
      return markSkipped.call(this, caseReport, "OPENROUTER_API_KEY is required");
    }

    let workspaceId = "";
    let podmanControl = null;
    try {
      const context = await prepareWorkspaceContext(caseReport, "lifecycle-runtime-resume");
      workspaceId = context.workspaceId;
      await runFirstTurnPhase(caseReport, context, "initial_first_turn");

      podmanControl = resolveDesktopPodmanControl();
      caseReport.runtime_control = {
        bin: podmanControl.bin,
        data_root: podmanControl.dataRoot,
        machine_name: podmanControl.machineName,
      };
      upsertCaseReport(caseReport);

      caseReport.diagnostics_before_runtime_stop = await collectLifecycleDiagnostics(workspaceId, podmanControl);
      upsertCaseReport(caseReport);

      stageLog(caseReport.id, "podman.machine.stop.start", podmanControl.machineName);
      caseReport.runtime_stop = await stopPodmanMachineAndWait(podmanControl);
      upsertCaseReport(caseReport);

      stageLog(caseReport.id, "podman.machine.start.start", podmanControl.machineName);
      caseReport.runtime_start = await startPodmanMachineAndWait(podmanControl);
      upsertCaseReport(caseReport);

      await openWorkspaceRouteAndWait(workspaceId);
      await assertConnectedLocalAndListening();
      await assertNoDaemonOverlayFor(15_000);
      caseReport.container.after_runtime_start = await getWorkspaceHarnessContainer(workspaceId);
      upsertCaseReport(caseReport);

      await runFirstTurnPhase(caseReport, context, "post_runtime_resume_first_turn");
      await verifyDaemonHealthPhase(caseReport, "post_runtime_resume", 15_000);

      caseReport.status = "passed";
      caseReport.completed_at = new Date().toISOString();
      upsertCaseReport(caseReport);
    } catch (error) {
      caseReport.status = "failed";
      caseReport.error = String(error);
      caseReport.completed_at = new Date().toISOString();
      caseReport.diagnostics_on_failure = await collectLifecycleDiagnostics(workspaceId, podmanControl)
        .catch((diagnosticError) => ({ error: String(diagnosticError) }));
      upsertCaseReport(caseReport);
      throw error;
    }
  }).timeout(CASE_TIMEOUT_MS);

  it("reopens an existing container workspace after local daemon and desktop app restart and completes the next first turn", async function () {
    const caseReport = {
      id: "local-app-daemon-resume",
      title: "daemon+app restart -> reopen workspace -> first turn succeeds",
      status: "running",
      started_at: new Date().toISOString(),
    };
    upsertCaseReport(caseReport);
    if (!scenarioEnabled("local-app-daemon-resume", ["lifecycle", "app-daemon-resume"])) {
      return markSkipped.call(this, caseReport, "scenario filter excluded app+daemon resume case");
    }
    const { apiKey } = readOpenRouterEnv();
    if (!apiKey) {
      // eslint-disable-next-line no-console
      console.error("[skip] OPENROUTER_API_KEY is required for container-lifecycle-resume.spec.cjs app-daemon-resume");
      return markSkipped.call(this, caseReport, "OPENROUTER_API_KEY is required");
    }

    let workspaceId = "";
    let podmanControl = null;
    try {
      const context = await prepareWorkspaceContext(caseReport, "lifecycle-app-daemon-resume");
      workspaceId = context.workspaceId;
      podmanControl = resolveDesktopPodmanControl();
      caseReport.runtime_control = {
        bin: podmanControl.bin,
        data_root: podmanControl.dataRoot,
        machine_name: podmanControl.machineName,
      };
      upsertCaseReport(caseReport);

      await runFirstTurnPhase(caseReport, context, "initial_first_turn");
      caseReport.diagnostics_before_restart = await collectLifecycleDiagnostics(workspaceId, podmanControl);
      upsertCaseReport(caseReport);

      stageLog(caseReport.id, "desktop.daemon.restart.start");
      caseReport.daemon_restart = await restartLocalDaemonAndWait();
      upsertCaseReport(caseReport);

      stageLog(caseReport.id, "desktop.app.restart.start");
      caseReport.app_restart = await restartDesktopAppAndWait();
      upsertCaseReport(caseReport);

      await openWorkspaceRouteAndWait(workspaceId);
      await assertConnectedLocalAndListening();
      await assertNoDaemonOverlayFor(20_000);
      caseReport.diagnostics_after_restart = await collectLifecycleDiagnostics(workspaceId, podmanControl);
      upsertCaseReport(caseReport);

      await runFirstTurnPhase(caseReport, context, "post_restart_first_turn");
      await verifyDaemonHealthPhase(caseReport, "post_restart", 15_000);

      caseReport.status = "passed";
      caseReport.completed_at = new Date().toISOString();
      upsertCaseReport(caseReport);
    } catch (error) {
      caseReport.status = "failed";
      caseReport.error = String(error);
      caseReport.completed_at = new Date().toISOString();
      caseReport.diagnostics_on_failure = await collectLifecycleDiagnostics(workspaceId, podmanControl)
        .catch((diagnosticError) => ({ error: String(diagnosticError) }));
      upsertCaseReport(caseReport);
      throw error;
    }
  }).timeout(CASE_TIMEOUT_MS);

  it("runs the combined lifecycle disruption contract when explicitly requested", async function () {
    const caseReport = {
      id: "nightly-lifecycle-combined",
      title: "runtime stop/start + daemon/app restart -> reopen workspace -> first turn succeeds",
      status: "running",
      started_at: new Date().toISOString(),
    };
    upsertCaseReport(caseReport);
    if (!explicitScenarioEnabled("nightly-lifecycle-combined")) {
      return markSkipped.call(this, caseReport, "combined lifecycle contract is nightly-only");
    }
    if (!podmanMachineSupported) {
      return markSkipped.call(this, caseReport, `podman machine lifecycle not supported on ${process.platform}`);
    }
    const { apiKey } = readOpenRouterEnv();
    if (!apiKey) {
      // eslint-disable-next-line no-console
      console.error("[skip] OPENROUTER_API_KEY is required for container-lifecycle-resume.spec.cjs nightly-combined");
      return markSkipped.call(this, caseReport, "OPENROUTER_API_KEY is required");
    }

    let workspaceId = "";
    let podmanControl = null;
    try {
      const context = await prepareWorkspaceContext(caseReport, "lifecycle-combined-nightly");
      workspaceId = context.workspaceId;
      podmanControl = resolveDesktopPodmanControl();
      caseReport.runtime_control = {
        bin: podmanControl.bin,
        data_root: podmanControl.dataRoot,
        machine_name: podmanControl.machineName,
      };
      upsertCaseReport(caseReport);

      await runFirstTurnPhase(caseReport, context, "initial_first_turn");

      caseReport.runtime_stop = await stopPodmanMachineAndWait(podmanControl);
      upsertCaseReport(caseReport);
      caseReport.runtime_start = await startPodmanMachineAndWait(podmanControl);
      upsertCaseReport(caseReport);

      await openWorkspaceRouteAndWait(workspaceId);
      await assertConnectedLocalAndListening();
      await assertNoDaemonOverlayFor(15_000);
      await runFirstTurnPhase(caseReport, context, "post_runtime_resume_first_turn");

      caseReport.daemon_restart = await restartLocalDaemonAndWait();
      upsertCaseReport(caseReport);
      caseReport.app_restart = await restartDesktopAppAndWait();
      upsertCaseReport(caseReport);

      await openWorkspaceRouteAndWait(workspaceId);
      await assertConnectedLocalAndListening();
      await assertNoDaemonOverlayFor(20_000);
      caseReport.diagnostics_after_combined_restart = await collectLifecycleDiagnostics(workspaceId, podmanControl);
      upsertCaseReport(caseReport);

      await runFirstTurnPhase(caseReport, context, "post_combined_restart_first_turn");
      await verifyDaemonHealthPhase(caseReport, "post_combined_restart", 15_000);

      caseReport.status = "passed";
      caseReport.completed_at = new Date().toISOString();
      upsertCaseReport(caseReport);
    } catch (error) {
      caseReport.status = "failed";
      caseReport.error = String(error);
      caseReport.completed_at = new Date().toISOString();
      caseReport.diagnostics_on_failure = await collectLifecycleDiagnostics(workspaceId, podmanControl)
        .catch((diagnosticError) => ({ error: String(diagnosticError) }));
      upsertCaseReport(caseReport);
      throw error;
    }
  }).timeout(CASE_TIMEOUT_MS);
});
