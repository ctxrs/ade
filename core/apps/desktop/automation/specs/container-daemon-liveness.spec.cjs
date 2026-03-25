const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson, sampleDaemonHealth } = require("./helpers/daemon.cjs");
const {
  getProviderStatus,
} = require("./helpers/provider_runtime.cjs");
const {
  mkTempDir,
  initGitRepo,
  runWizardScenario,
  getWorkspace,
  getWorkspaceHarnessContainer,
  assertLocalWorkspaceConfig,
  assertConnectedLocalAndListening,
  assertWorkspaceTerminalCwdPrefix,
  assertNoDaemonOverlayFor,
  collectWorkspaceRouteDiagnostics,
} = require("./helpers/workspace_wizard_flow.cjs");
const { openWorkspaceRouteAndWait } = require("./helpers/container_lifecycle.cjs");

const scenarioFilter = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean),
);

const scenarioEnabled = (name, tags = []) => {
  if (scenarioFilter.size === 0) return true;
  return [name, ...tags].some((t) => scenarioFilter.has(String(t).trim().toLowerCase()));
};
const CASE_TIMEOUT_MS = Number.parseInt(
  String(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "900000"),
  10,
) || 900000;
const ACP_BRIDGE_PROVIDER_ID = "acp-crp-bridge";
const ACP_BRIDGE_INVALID_PATTERNS = [
  /ACP bridge runtime is not configured or invalid/i,
  /runtime command is not configured for provider 'acp-crp-bridge'/i,
];

const hasAcpBridgeInvalidDiagnostic = (status) => {
  const diagnostics = Array.isArray(status?.diagnostics) ? status.diagnostics : [];
  return diagnostics.some((entry) => ACP_BRIDGE_INVALID_PATTERNS.some((pattern) => pattern.test(String(entry || ""))));
};

const readLaunchPanelDiagnostics = async () => {
  return await browser.execute(() => {
    const panel = document.querySelector('[data-testid="wizard-launch-log-panel"]');
    const copyButton = document.querySelector('[data-testid="wizard-launch-copy"]');
    const lines = Array.from(document.querySelectorAll(".wizard-launch-log-line")).map((row) => {
      const ts = String(row.querySelector(".wizard-launch-log-ts")?.textContent || "").trim();
      const phase = String(row.querySelector(".wizard-launch-log-phase")?.textContent || "").trim();
      const level = String(row.querySelector(".wizard-launch-log-level")?.textContent || "").trim();
      const message = String(row.querySelector(".wizard-launch-log-msg")?.textContent || "").trim();
      return { ts, phase, level, message };
    });
    return {
      hasPanel: Boolean(panel),
      copyLabel: copyButton ? String(copyButton.textContent || "").trim() : "",
      lines,
    };
  });
};

const waitForLaunchPanelLogMessage = async (message, timeoutMs = 30_000, pollMs = 250) => {
  const messages = Array.isArray(message) ? message : [message];
  const started = Date.now();
  let lastLaunch = null;
  while (Date.now() - started < timeoutMs) {
    lastLaunch = await readLaunchPanelDiagnostics();
    if (
      Array.isArray(lastLaunch?.lines)
      && lastLaunch.lines.some((line) => messages.includes(String(line?.message || "").trim()))
    ) {
      return lastLaunch;
    }
    await browser.pause(pollMs);
  }
  throw new Error(`launch panel never surfaced ${JSON.stringify(messages)}: ${JSON.stringify(lastLaunch)}`);
};

const readStartupPrewarmDiagnostics = async () => {
  const diagnostics = await daemonJson("GET", "/api/diagnostics");
  return {
    diagnostics,
    startupPrewarm: diagnostics.payload?.execution?.startup_prewarm || null,
  };
};

const ensureExecutionMachineReady = async ({
  dest,
  name,
  timeoutMs = 15 * 60_000,
}) => {
  const initial = await readStartupPrewarmDiagnostics();
  if (initial.diagnostics.status === 200 && initial.startupPrewarm?.machine_ready === true) {
    return {
      bootstrap: null,
      startupPrewarm: initial.startupPrewarm,
    };
  }

  const bootstrap = await startBackgroundContainerLaunchJob({
    dest,
    name,
  });
  const bootstrapFinal = await waitForLaunchTerminalState(bootstrap.jobId, timeoutMs);
  if (bootstrapFinal.state !== "ready") {
    throw new Error(`machine bootstrap launch did not finish cleanly: ${JSON.stringify(bootstrapFinal)}`);
  }
  return {
    bootstrap,
    startupPrewarm: initial.startupPrewarm,
  };
};

const startBackgroundContainerLaunchJob = async ({
  dest,
  name,
  environment = "host",
  networkMode = "all",
}) => {
  initGitRepo(dest, name);

  const create = await daemonJson("POST", "/api/workspaces", {
    root_path: dest,
    name,
  });
  if (create.status !== 200 || !create.payload?.id) {
    throw new Error(`failed to create background workspace for launch contention: ${JSON.stringify(create)}`);
  }

  const workspaceId = String(create.payload.id || "").trim();
  const config = await daemonJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment,
    network_mode: networkMode,
  });
  if (config.status !== 200) {
    throw new Error(`failed to configure background workspace execution: ${JSON.stringify(config)}`);
  }

  const launch = await daemonJson("POST", "/api/execution/launch/start", {
    workspace_id: workspaceId,
  });
  if (launch.status !== 200 || !launch.payload?.job_id) {
    throw new Error(`failed to start background workspace launch: ${JSON.stringify(launch)}`);
  }

  return {
    workspaceId,
    jobId: String(launch.payload.job_id || "").trim(),
  };
};

const startBackgroundRuntimePrewarmJob = async ({ scope = "all" } = {}) => {
  const launch = await daemonJson("POST", "/api/execution/launch/start", {
    kind: "startup_prewarm",
    prewarm_scope: scope,
  });
  if (launch.status !== 200 || !launch.payload?.job_id) {
    throw new Error(`failed to start background runtime prewarm: ${JSON.stringify(launch)}`);
  }
  return {
    jobId: String(launch.payload.job_id || "").trim(),
  };
};

const readLaunchSnapshot = async (jobId) => {
  const response = await daemonJson("GET", `/api/execution/launch/status?job_id=${encodeURIComponent(jobId)}`);
  if (response.status !== 200) {
    throw new Error(`failed to read launch status for ${jobId}: ${JSON.stringify(response)}`);
  }
  return response.payload;
};

const waitForLaunchTerminalState = async (jobId, timeoutMs = 600_000, pollMs = 500) => {
  const started = Date.now();
  let lastSnapshot = null;
  while (Date.now() - started < timeoutMs) {
    lastSnapshot = await readLaunchSnapshot(jobId);
    if (lastSnapshot?.state === "ready" || lastSnapshot?.state === "error") {
      return lastSnapshot;
    }
    await browser.pause(pollMs);
  }
  throw new Error(`launch job ${jobId} did not reach terminal state: ${JSON.stringify(lastSnapshot)}`);
};

const waitForLaunchLogMessage = async (jobId, message, timeoutMs = 120_000, pollMs = 500) => {
  const messages = Array.isArray(message) ? message : [message];
  const started = Date.now();
  let lastSnapshot = null;
  while (Date.now() - started < timeoutMs) {
    lastSnapshot = await readLaunchSnapshot(jobId);
    if (
      Array.isArray(lastSnapshot?.logs)
      && lastSnapshot.logs.some((line) => messages.includes(String(line?.message || "").trim()))
    ) {
      return lastSnapshot;
    }
    await browser.pause(pollMs);
  }
  throw new Error(`launch log ${JSON.stringify(messages)} never surfaced for ${jobId}: ${JSON.stringify(lastSnapshot)}`);
};

const waitForQueuedRuntimePrewarmJob = async (jobIds, timeoutMs = 30_000, pollMs = 100) => {
  const candidates = Array.isArray(jobIds) ? jobIds.filter(Boolean) : [];
  if (candidates.length === 0) {
    throw new Error("expected queued runtime prewarm jobs, got none");
  }

  const started = Date.now();
  let lastSnapshots = [];
  while (Date.now() - started < timeoutMs) {
    lastSnapshots = await Promise.all(candidates.map(async (jobId) => ({
      jobId,
      snapshot: await readLaunchSnapshot(jobId),
    })));
    const waiting = lastSnapshots.find(({ snapshot }) => {
      const logs = Array.isArray(snapshot?.logs) ? snapshot.logs : [];
      const sawWaiting = logs.some((line) => line?.message === "waiting for runtime prewarm slot");
      const sawAcquired = logs.some((line) => line?.message === "runtime prewarm slot acquired");
      return snapshot?.state === "running" && sawWaiting && !sawAcquired;
    });
    if (waiting) return waiting;
    await browser.pause(pollMs);
  }

  throw new Error(
    `runtime prewarm queue never formed before create: ${JSON.stringify(lastSnapshots.map(({ jobId, snapshot }) => ({
      jobId,
      state: snapshot?.state || null,
      logs: Array.isArray(snapshot?.logs) ? snapshot.logs.slice(-5) : [],
    })))}`,
  );
};

const prepareRuntimePrewarmContention = async () => {
  const holder = await startBackgroundRuntimePrewarmJob();
  await waitForLaunchLogMessage(holder.jobId, "runtime prewarm slot acquired");
  const queued = await startBackgroundRuntimePrewarmJob();
  await waitForQueuedRuntimePrewarmJob([queued.jobId]);
  return {
    jobs: [holder, queued],
    holderJobId: holder.jobId,
    queuedJobId: queued.jobId,
  };
};

const waitForProviderInstallOrAcpBridgeObservation = async (
  providerId,
  target,
  { timeoutMs = 10 * 60_000, pollMs = 2_000, settleMs = 5_000 } = {},
) => {
  const startedAt = Date.now();
  let lastProviderStatus = null;
  let lastBridgeStatus = null;
  while (Date.now() - startedAt < timeoutMs) {
    lastProviderStatus = await getProviderStatus(providerId, target);
    lastBridgeStatus = await getProviderStatus(ACP_BRIDGE_PROVIDER_ID, target);

    if (lastProviderStatus.installed) {
      if (lastBridgeStatus.installed && lastBridgeStatus.health === "ok") {
        return {
          outcome: "viable",
          providerStatus: lastProviderStatus,
          bridgeStatus: lastBridgeStatus,
        };
      }
      throw new Error(
        `provider '${providerId}' installed but ACP bridge runtime stayed invalid for target=${target}: ${JSON.stringify(lastBridgeStatus)}`,
      );
    }

    if (hasAcpBridgeInvalidDiagnostic(lastProviderStatus) || hasAcpBridgeInvalidDiagnostic(lastBridgeStatus)) {
      return {
        outcome: "acp-bridge-invalid",
        providerStatus: lastProviderStatus,
        bridgeStatus: lastBridgeStatus,
      };
    }

    const installRunning = lastProviderStatus.details.install_running === "true";
    if (!installRunning && Date.now() - startedAt >= settleMs) {
      const detail = lastProviderStatus.diagnostics[0]
        || lastBridgeStatus.diagnostics[0]
        || `provider=${JSON.stringify(lastProviderStatus)} bridge=${JSON.stringify(lastBridgeStatus)}`;
      throw new Error(
        `provider '${providerId}' did not converge to install or explicit ACP bridge diagnostics for target=${target}: ${detail}`,
      );
    }
    await browser.pause(pollMs);
  }

  throw new Error(
    `provider '${providerId}' install/ACP bridge observation timed out for target=${target}: ${JSON.stringify({
      providerStatus: lastProviderStatus,
      bridgeStatus: lastBridgeStatus,
    })}`,
  );
};

const runLocalContainerCreate = async ({ container, workspaceName, destPath, network = "full" }) => {
  try {
    return await runWizardScenario({
      location: "local",
      container,
      network,
      networkAllowlist: network === "allowlist" ? "github.com\nregistry.npmjs.org" : "",
      harnessDownloads: "skip",
      source: { kind: "new", destPath, workspaceName },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });
  } catch (error) {
    const launch = await readLaunchPanelDiagnostics();
    const routeDiag = await collectWorkspaceRouteDiagnostics();
    const hasDetailedError = launch.lines.some((line) => line.level.toLowerCase() === "error" && line.message.length > 8);
    if (!hasDetailedError) {
      throw new Error(
        `sandbox launch failed without detailed launch diagnostics; error=${String(error)} launch=${JSON.stringify(launch)} route=${JSON.stringify(routeDiag)}`,
      );
    }
    throw new Error(
      `sandbox launch failed; error=${String(error)} launch=${JSON.stringify(launch)} route=${JSON.stringify(routeDiag)}`,
    );
  }
};

describe("sandbox daemon liveness", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-container-daemon-liveness-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?containerDaemonLiveness=${Date.now()}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("surfaces startup prewarm contention through the desktop create flow", async function () {
    if (
      !scenarioEnabled("local-new-host", ["local", "host"])
      && !scenarioEnabled("local-new-sandbox", ["local", "sandbox"])
      && !scenarioEnabled("local-mixed-mode", ["local", "mixed-mode"])
    ) {
      this.skip();
    }

    await assertConnectedLocalAndListening();
    const dest = path.join(localBase, "prewarm-contention");
    let prewarmQueue = null;
    let launch = null;
    const workspaceId = await runWizardScenario({
      location: "local",
      container: "host",
      network: "allowlist",
      networkAllowlist: "github.com\nregistry.npmjs.org",
      harnessDownloads: "skip",
      source: { kind: "new", destPath: dest, workspaceName: "prewarm-contention" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
      beforeCreate: async () => {
        await ensureExecutionMachineReady({
          dest: path.join(localBase, "prewarm-bootstrap"),
          name: "prewarm-bootstrap",
        });
        prewarmQueue = await prepareRuntimePrewarmContention();
      },
      onLaunchLogsVisible: async () => {
        launch = await waitForLaunchPanelLogMessage("waiting for runtime prewarm slot");
      },
    });

    if (!launch || !Array.isArray(launch.lines) || launch.lines.length === 0) {
      throw new Error(`workspace launch never exposed prewarm contention diagnostics: ${JSON.stringify(launch)}`);
    }
    const contentionLine = launch.lines.find((line) => line.message === "waiting for runtime prewarm slot");
    if (!contentionLine) {
      throw new Error(`workspace launch missed prewarm contention log: ${JSON.stringify(launch)}`);
    }
    if (!prewarmQueue || !Array.isArray(prewarmQueue.jobs) || prewarmQueue.jobs.length < 2) {
      throw new Error(`background runtime prewarm queue missing from contention setup: ${JSON.stringify(prewarmQueue)}`);
    }
    const prewarmFinals = await Promise.all(
      prewarmQueue.jobs.map(async ({ jobId }) => await waitForLaunchTerminalState(jobId)),
    );
    const failedPrewarms = prewarmFinals.filter((snapshot) => snapshot.state !== "ready");
    if (failedPrewarms.length > 0) {
      throw new Error(`background runtime prewarm queue did not finish cleanly: ${JSON.stringify(failedPrewarms)}`);
    }

    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "host",
      networkMode: "allowlist",
      allowlist: ["github.com", "registry.npmjs.org"],
    });
    const container = await getWorkspaceHarnessContainer(workspaceId);
    if (!container || !container.running) {
      throw new Error(`expected running host execution harness after contention create, got ${JSON.stringify(container)}`);
    }
  }).timeout(CASE_TIMEOUT_MS);

  it("local host workspace create keeps daemon healthy", async function () {
    if (
      !scenarioEnabled("local-new-host", ["local", "host"])
      && !scenarioEnabled("local-codex-smoke", ["local", "host", "provider"])
    ) {
      this.skip();
    }

    const dest = path.join(localBase, "host");
    const workspaceId = await runLocalContainerCreate({
      container: "host",
      workspaceName: "container-daemon-host",
      destPath: dest,
      network: "allowlist",
    });

    await assertConnectedLocalAndListening();
    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "host",
      networkMode: "allowlist",
      allowlist: ["github.com", "registry.npmjs.org"],
    });
    const container = await getWorkspaceHarnessContainer(workspaceId);
    if (!container || !container.running) {
      throw new Error(`expected running host execution harness, got ${JSON.stringify(container)}`);
    }
    await assertNoDaemonOverlayFor(20_000);
    const health = await sampleDaemonHealth({ durationMs: 20_000, intervalMs: 2_000 });
    if (!health.ok) {
      throw new Error(`daemon health degraded after host create: ${JSON.stringify(health.failures)}`);
    }
  }).timeout(CASE_TIMEOUT_MS);

  it("local sandbox workspace create keeps daemon healthy", async function () {
    if (!scenarioEnabled("local-new-sandbox", ["local", "sandbox"])) this.skip();

    const dest = path.join(localBase, "sandbox");
    const workspaceId = await runWizardScenario({
      location: "local",
      container: "sandbox",
      network: "full",
      harnessDownloads: true,
      selectedHarnessProviderIds: ["cursor"],
      requireSelectedHarnessInstallsNonBlocking: true,
      requireExactSelectedHarnessInstallProof: true,
      source: { kind: "new", destPath: dest, workspaceName: "container-daemon-sandbox" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    await assertConnectedLocalAndListening();
    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "sandbox",
      networkMode: "all",
    });
    await waitForProviderInstallOrAcpBridgeObservation("cursor", "container", {
      timeoutMs: 10 * 60_000,
      pollMs: 2_000,
    });
    const container = await getWorkspaceHarnessContainer(workspaceId);
    if (!container || !container.running) {
      throw new Error(`expected running sandbox environment, got ${JSON.stringify(container)}`);
    }

    const tasksResp = await daemonJson("GET", `/api/workspaces/${workspaceId}/tasks`);
    if (tasksResp.status !== 200) {
      throw new Error(`failed to read workspace tasks after create (${tasksResp.status})`);
    }

    await assertNoDaemonOverlayFor(30_000);
    const health = await sampleDaemonHealth({ durationMs: 30_000, intervalMs: 2_000 });
    if (!health.ok) {
      throw new Error(`daemon health degraded after sandbox create: ${JSON.stringify(health.failures)}`);
    }
  }).timeout(CASE_TIMEOUT_MS);

  it("same-daemon host and sandbox workspaces both stay routable", async function () {
    if (!scenarioEnabled("local-mixed-mode", ["local", "mixed-mode"])) this.skip();

    const hostDest = path.join(localBase, "mixed-host");
    const hostWorkspaceId = await runWizardScenario({
      location: "local",
      container: "host",
      source: { kind: "new", destPath: hostDest, workspaceName: "mixed-host" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    await assertConnectedLocalAndListening();
    const hostWorkspace = await getWorkspace(hostWorkspaceId);
    await assertLocalWorkspaceConfig(hostWorkspaceId, {
      environment: "host",
    });
    await assertWorkspaceTerminalCwdPrefix(hostWorkspaceId, hostWorkspace.root_path);

    const diskDest = path.join(localBase, "mixed-sandbox");
    const diskWorkspaceId = await runLocalContainerCreate({
      container: "sandbox",
      workspaceName: "mixed-sandbox",
      destPath: diskDest,
      network: "full",
    });

    await assertConnectedLocalAndListening();
    await assertLocalWorkspaceConfig(diskWorkspaceId, {
      environment: "sandbox",
      networkMode: "all",
    });
    const diskContainer = await getWorkspaceHarnessContainer(diskWorkspaceId);
    if (!diskContainer || !diskContainer.running) {
      throw new Error(`expected running sandbox harness, got ${JSON.stringify(diskContainer)}`);
    }
    await assertWorkspaceTerminalCwdPrefix(diskWorkspaceId, "/ctx/ws");

    const workspacesResp = await daemonJson("GET", "/api/workspaces");
    if (workspacesResp.status !== 200 || !Array.isArray(workspacesResp.payload)) {
      throw new Error(`failed to list workspaces after mixed-mode create (${workspacesResp.status})`);
    }
    const workspaceIds = new Set(workspacesResp.payload.map((workspace) => String(workspace?.id || "")));
    if (!workspaceIds.has(String(hostWorkspaceId)) || !workspaceIds.has(String(diskWorkspaceId))) {
      throw new Error(
        `expected both mixed-mode workspaces to remain registered; ids=${JSON.stringify(Array.from(workspaceIds))}`,
      );
    }

    await openWorkspaceRouteAndWait(hostWorkspaceId);
    await assertNoDaemonOverlayFor(5_000);
    await assertWorkspaceTerminalCwdPrefix(hostWorkspaceId, hostWorkspace.root_path);

    await openWorkspaceRouteAndWait(diskWorkspaceId);
    await assertNoDaemonOverlayFor(5_000);
    await assertWorkspaceTerminalCwdPrefix(diskWorkspaceId, "/ctx/ws");

    const health = await sampleDaemonHealth({ durationMs: 20_000, intervalMs: 2_000 });
    if (!health.ok) {
      throw new Error(`daemon health degraded after same-daemon mixed-mode create: ${JSON.stringify(health.failures)}`);
    }
  }).timeout(CASE_TIMEOUT_MS);
});
