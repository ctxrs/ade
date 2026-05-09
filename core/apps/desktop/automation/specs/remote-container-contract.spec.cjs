const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const { tauriInvoke, connectSshWithPolling } = require("./helpers/desktop_connection.cjs");
const { navigateToTauriUrl } = require("./helpers/tauri.cjs");
const { daemonJson, safeDaemonJson } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  runWizardScenario,
  getWorkspace,
  assertWorkspaceTerminalCwdPrefix,
  assertNoDaemonOverlayFor,
  collectWorkspaceRouteDiagnostics,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  ensureCodexOpenRouterWorkspaceReady,
  getProviderStatus,
  installManagedProvidersAndAssertInstalled,
  resolveManagedProviderInstallIds,
  verifyProviderForWorkspace,
} = require("./helpers/provider_runtime.cjs");
const { runDeterministicFirstTurnOutcome } = require("./helpers/first_turn_contract.cjs");
const {
  createRemoteContractRecorder,
  parseBoolean,
  resolveRemoteFixtureEnv,
  resolveRemotePerformanceBudgets,
} = require("../helpers/remote_fixture_contract.cjs");
const { expectedRemoteRootPrefix } = require("../helpers/remote_container_contract_paths.cjs");

const REMOTE_CTX_BIN = "$HOME/.ctx/bin/ctx";
const reportPath = String(
  process.env.CTX_REMOTE_CONTAINER_CONTRACT_REPORT || path.join("/tmp", "ctx-remote-container-contract.json"),
).trim();
const REQUIRE_FIRST_TURN_SUCCESS = parseBoolean(process.env.CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS || "0");
const REQUIRE_ALL_HARNESS_INSTALLS = parseBoolean(process.env.CTX_REMOTE_WORKSPACE_E2E_REQUIRE_ALL_HARNESS_INSTALLS || "0");
const SKIP_MANAGED_BINARY_RESET = parseBoolean(process.env.CTX_AUTOMATION_REMOTE_SKIP_MANAGED_BINARY_RESET || "0");
const fixture = resolveRemoteFixtureEnv({ lane: "sandbox" });
const perfBudgets = resolveRemotePerformanceBudgets({ fixture });
const scenarioFilter = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((entry) => entry.trim().toLowerCase())
    .filter(Boolean),
);

const scenarioEnabled = (name, tags = []) => {
  if (scenarioFilter.size === 0) return true;
  return [name, ...tags].some((entry) => scenarioFilter.has(String(entry).trim().toLowerCase()));
};

let contractRecorder = null;

const recordTimedOperation = async (name, fn) => {
  const startedAt = Date.now();
  const value = await fn();
  const measurement = {
    name,
    started_at: new Date(startedAt).toISOString(),
    elapsed_ms: Date.now() - startedAt,
    value,
  };
  contractRecorder?.recordArtifact(name, measurement);
  return measurement;
};

const assertElapsedWithinBudget = (name, measurement, maxMs) => {
  if (!measurement || typeof measurement.elapsed_ms !== "number") {
    throw new Error(`${name}: missing elapsed_ms measurement`);
  }
  if (measurement.elapsed_ms > maxMs) {
    throw new Error(`${name} exceeded ${maxMs}ms (actual ${measurement.elapsed_ms}ms)`);
  }
  contractRecorder?.recordAssertion(
    name,
    "pass",
    `${name} completed in ${measurement.elapsed_ms}ms (budget ${maxMs}ms)`,
  );
};

const sshBaseArgs = ({ useKey = true } = {}) => {
  const args = [
    "-o",
    "StrictHostKeyChecking=no",
    "-o",
    "UserKnownHostsFile=/dev/null",
    "-o",
    "ConnectTimeout=10",
  ];
  if (fixture.sshConfigPath) {
    args.unshift(fixture.sshConfigPath);
    args.unshift("-F");
  }
  if (fixture.sshPort > 0) {
    args.push("-p", String(fixture.sshPort));
  }
  if (useKey && fixture.sshKeyPath) {
    args.unshift("IdentitiesOnly=yes");
    args.unshift("-o");
    args.unshift(fixture.sshKeyPath);
    args.unshift("-i");
  }
  return args;
};

const remoteSsh = (command, { auth = "key", label = "ssh" } = {}) => {
  const usePassword = auth === "password";
  const args = sshBaseArgs({ useKey: !usePassword });
  const remoteCommand = `bash -lc ${JSON.stringify(command)}`;
  let binary = "ssh";
  let finalArgs;
  let env = process.env;

  if (usePassword) {
    const password = String(fixture.passwordActual || fixture.password || "").trim();
    if (!password) {
      throw new Error("password auth requested but the remote sandbox fixture password is not set");
    }
    binary = "sshpass";
    finalArgs = [
      "-e",
      "ssh",
      ...args,
      "-o",
      "BatchMode=no",
      "-o",
      "PreferredAuthentications=password,keyboard-interactive",
      "-o",
      "NumberOfPasswordPrompts=1",
      fixture.target,
      remoteCommand,
    ];
    env = { ...process.env, SSHPASS: password };
  } else {
    finalArgs = [...args, "-o", "BatchMode=yes", fixture.target, remoteCommand];
  }

  const result = spawnSync(binary, finalArgs, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    env,
  });
  contractRecorder?.recordSshTranscript(label, {
    auth,
    binary,
    args: finalArgs,
    command,
    remote_command: remoteCommand,
    target: fixture.target,
    exit_code: result.status ?? null,
    signal: result.signal ?? null,
    ok: result.status === 0,
    stdout: String(result.stdout || "").trim(),
    stderr: String(result.stderr || "").trim(),
  });
  if (result.status !== 0) {
    const stderr = String(result.stderr || "").trim();
    const stdout = String(result.stdout || "").trim();
    const detail = [
      `command failed: ${binary} ${finalArgs.join(" ")}`,
      stderr ? `stderr: ${stderr}` : null,
      stdout ? `stdout: ${stdout}` : null,
    ]
      .filter(Boolean)
      .join("\n");
    throw new Error(detail);
  }
  return String(result.stdout || "").trim();
};

const readRemoteBinaryFingerprint = (auth) => {
  const cmd = [
    "set -eu",
    `if [ -x ${REMOTE_CTX_BIN} ]; then set -- $(cksum ${REMOTE_CTX_BIN}); mtime=$(stat -c %Y ${REMOTE_CTX_BIN} 2>/dev/null || echo 0); printf '%s|%s|%s\\n' \"$1\" \"$2\" \"$mtime\"; else printf 'missing\\n'; fi`,
  ].join("; ");
  const raw = remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, {
    auth,
    label: "managed-binary-fingerprint",
  }).trim();
  if (!raw || raw === "missing") {
    return { present: false };
  }
  const [checksum, bytes, mtime] = raw.split("|");
  return {
    present: true,
    checksum: String(checksum || "").trim(),
    bytes: Number.parseInt(String(bytes || "0"), 10) || 0,
    mtime: Number.parseInt(String(mtime || "0"), 10) || 0,
  };
};

const readRemoteDaemonFingerprint = (auth) => {
  const cmd = [
    "set -eu",
    "pid=$(pgrep -xo ctx || true)",
    "if [ -n \"$pid\" ]; then etimes=$(ps -o etimes= -p \"$pid\" 2>/dev/null | tr -d ' '); started=$(ps -o lstart= -p \"$pid\" 2>/dev/null | sed 's/^ *//'); printf '%s|%s|%s\\n' \"$pid\" \"${etimes:-0}\" \"$started\"; else printf 'missing\\n'; fi",
  ].join("; ");
  const raw = remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, {
    auth,
    label: "remote-daemon-fingerprint",
  }).trim();
  if (!raw || raw === "missing") {
    return { present: false };
  }
  const [pid, etimes, ...startedParts] = raw.split("|");
  return {
    present: true,
    pid: String(pid || "").trim(),
    etimes: Number.parseInt(String(etimes || "0"), 10) || 0,
    started: startedParts.join("|").trim(),
  };
};

const collectRemoteRuntimeState = (auth, label) => {
  const snapshot = {
    binary: readRemoteBinaryFingerprint(auth),
    daemon: readRemoteDaemonFingerprint(auth),
  };
  contractRecorder?.recordArtifact(label, snapshot);
  return snapshot;
};

const assertSameBinaryFingerprint = (before, after, label) => {
  if (!before?.present || !after?.present) {
    throw new Error(`${label}: expected managed binary to be present before and after reconnect`);
  }
  const changed = (
    before.checksum !== after.checksum
    || before.bytes !== after.bytes
    || before.mtime !== after.mtime
  );
  if (changed) {
    throw new Error(
      `${label}: managed binary changed unexpectedly: before=${JSON.stringify(before)} after=${JSON.stringify(after)}`,
    );
  }
  contractRecorder?.recordAssertion(label, "pass", "managed binary fingerprint stayed stable");
};

const recordDaemonProcessObservation = (before, after, label) => {
  if (!before?.present && !after?.present) {
    contractRecorder?.recordAssertion(
      label,
      "pass",
      "remote daemon process was not directly observable across reconnect; health and workspace continuity remain the contract",
    );
    return;
  }
  if (before?.present && after?.present && before.pid === after.pid && before.started === after.started) {
    contractRecorder?.recordAssertion(label, "pass", "daemon process fingerprint stayed stable");
    return;
  }
  contractRecorder?.recordAssertion(
    label,
    "pass",
    `remote daemon process changed or was only partially observable across reconnect: before=${JSON.stringify(before)} after=${JSON.stringify(after)}`,
  );
};

const createTaskSmoke = async (workspaceId, label) => {
  const resp = await daemonJson("POST", `/api/workspaces/${workspaceId}/tasks`, {
    title: `${label}-${Date.now()}`,
    description: `Remote sandbox warm task smoke for ${label}`,
  });
  if (resp.status !== 200 && resp.status !== 201) {
    throw new Error(`task create failed (${resp.status}): ${JSON.stringify(resp.payload || null)}`);
  }
  return resp.payload || null;
};

const assertProviderInstalledNoInstallRunning = async (target, label) => {
  const status = await getProviderStatus("codex", target);
  contractRecorder?.recordArtifact(`${label}_provider_status`, status);
  if (!status.installed) {
    throw new Error(`${label}: codex provider is not installed for target=${target}`);
  }
  if (String(status.details.install_running || "").trim().toLowerCase() === "true") {
    throw new Error(`${label}: unexpected provider install still running for target=${target}`);
  }
  return status;
};

const assertWarmWorkspaceUsability = async ({
  workspaceId,
  installTarget,
  endpointName,
  perfLabel,
}) => {
  await assertProviderInstalledNoInstallRunning(installTarget, `${perfLabel}_before`);

  const providerReady = await recordTimedOperation(`${perfLabel}_provider_ready`, async () => {
    const provider = await ensureCodexOpenRouterWorkspaceReady(workspaceId, {
      installTarget,
      endpointName,
      allowInstall: false,
      timeoutMs: perfBudgets.provider_ready_ms,
      pollMs: 2000,
    });
    const verifyPayload = await verifyProviderForWorkspace(workspaceId, provider.providerId);
    return {
      provider,
      verifyPayload,
    };
  });
  assertElapsedWithinBudget(`${perfLabel}_provider_ready_budget`, providerReady, perfBudgets.provider_ready_ms);

  const terminalReady = await recordTimedOperation(`${perfLabel}_terminal_ready`, async () => {
    await assertWorkspaceTerminalCwdPrefix(workspaceId, "/ctx/ws");
    return { cwd_prefix: "/ctx/ws" };
  });
  assertElapsedWithinBudget(`${perfLabel}_terminal_ready_budget`, terminalReady, perfBudgets.terminal_ready_ms);

  const taskCreate = await recordTimedOperation(`${perfLabel}_task_create`, async () => {
    return await createTaskSmoke(workspaceId, perfLabel);
  });
  assertElapsedWithinBudget(`${perfLabel}_task_create_budget`, taskCreate, perfBudgets.task_create_ms);

  await assertProviderInstalledNoInstallRunning(installTarget, `${perfLabel}_after`);
  contractRecorder?.recordAssertion(
    `${perfLabel}_workspace_usable`,
    "pass",
    "provider verify, terminal create, and task create succeeded without reinstall or reprovision",
  );
};

const collectFailureArtifacts = async (stage) => {
  if (!contractRecorder) return;

  contractRecorder.recordArtifact(`${stage}_workspace_route`, await collectWorkspaceRouteDiagnostics());
  contractRecorder.recordArtifact(`${stage}_daemon_health`, await safeDaemonJson("GET", "/api/health"));
  contractRecorder.recordArtifact(`${stage}_daemon_diagnostics`, await safeDaemonJson("GET", "/api/diagnostics"));

  if (!fixture.target || !fixture.dataDir) return;

  const remoteLogFile = `${fixture.dataDir.replace(/\/+$/, "")}/logs/daemon.log`;
  const tailCmd = `if [ -f ${JSON.stringify(remoteLogFile)} ]; then tail -n 200 ${JSON.stringify(remoteLogFile)}; else echo "__CTX_MISSING__ ${remoteLogFile}"; fi`;
  try {
    const tail = remoteSsh(`sh -lc ${JSON.stringify(tailCmd)}`, {
      auth: fixture.authMode === "password" ? "password" : "key",
      label: `${stage}-remote-daemon-log-tail`,
    });
    contractRecorder.recordArtifact(`${stage}_remote_daemon_log_tail`, {
      log_file: remoteLogFile,
      tail,
    });
  } catch (error) {
    contractRecorder.recordArtifact(`${stage}_remote_daemon_log_tail`, {
      log_file: remoteLogFile,
      error: String(error),
    });
  }
};

describe("remote sandbox contract (env-gated desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-remote-container-contract-${runId}-`);
  const remoteBase = `/tmp/ctx-remote-contract-${runId}`;
  const remoteDataDir = fixture.dataDir || `${remoteBase}/daemon`;
  const remoteAuthMode = fixture.authMode === "password" ? "password" : "key";
  const reconnectReq = () => ({
    host: fixture.host,
    user: fixture.user,
    remote_port: fixture.port,
    start_remote: true,
    remote_data_dir: remoteDataDir || undefined,
    password_once: remoteAuthMode === "password"
      ? (fixture.passwordActual || fixture.password || undefined)
      : undefined,
  });

  const managedBinaryState = () =>
    remoteSsh(`if [ -x ${REMOTE_CTX_BIN} ]; then echo present; else echo missing; fi`, {
      auth: remoteAuthMode,
      label: "managed-state",
    }).trim();

  const assertDesktopSshConnection = async (stage) => {
    const connectionResp = await tauriInvoke("desktop_get_connection", {});
    contractRecorder?.recordArtifact(`desktop_connection_${stage}`, connectionResp);
    if (connectionResp.error) {
      throw new Error(`${stage}: desktop_get_connection failed: ${connectionResp.error}`);
    }
    const kind = String(connectionResp.value?.kind || "").toLowerCase();
    if (kind !== "ssh") {
      throw new Error(`${stage}: expected ssh desktop connection, got ${JSON.stringify(connectionResp.value || null)}`);
    }
    return connectionResp.value || null;
  };

  const resetRemoteBootstrapState = () => {
    if (SKIP_MANAGED_BINARY_RESET) {
      remoteSsh(`set -euo pipefail; mkdir -p ${JSON.stringify(remoteBase)}`, {
        auth: remoteAuthMode,
        label: "remote-bootstrap-preserve",
      });
      return;
    }
    remoteSsh(
      [
        "set -euo pipefail",
        "if command -v pkill >/dev/null 2>&1; then pkill -x ctx >/dev/null 2>&1 || true; fi",
        `rm -f ${REMOTE_CTX_BIN}`,
        `rm -rf ${JSON.stringify(remoteDataDir)}`,
        `rm -rf ${JSON.stringify(remoteBase)}`,
        `mkdir -p ${JSON.stringify(remoteBase)}`,
      ].join("; "),
      {
        auth: remoteAuthMode,
        label: "remote-bootstrap-reset",
      },
    );
  };

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }

    if (!fixture.ready || !fixture.target) return;

    try {
      remoteSsh(`set -euo pipefail; rm -rf ${JSON.stringify(remoteBase)}`, {
        auth: fixture.authMode === "password" ? "password" : "key",
        label: "suite-cleanup",
      });
    } catch {
      // ignore cleanup failures
    }
  });

  it("creates remote sandbox workspace when remote env is present", async function () {
    this.timeout(REQUIRE_ALL_HARNESS_INSTALLS ? 45 * 60_000 : 12 * 60_000);

    contractRecorder = createRemoteContractRecorder({
      outputPath: reportPath,
      suite: "remote sandbox contract",
      lane: "sandbox",
      fixture,
      secretValues: [fixture.password, fixture.passwordActual, process.env.OPENROUTER_API_KEY],
    });

    let finalized = false;
    const finalizeReport = (payload) => {
      if (finalized) return null;
      finalized = true;
      return contractRecorder.finalize(payload);
    };

    let workspaceId = "";
    let rootPath = "";
    let provider = null;
    let firstTurn = { attempted: false };
    let finalResult = "passed";
    let finalReason = "remote sandbox contract validated";
    let finalError = "";

    contractRecorder.recordArtifact("fixture_preflight", {
      report_path: reportPath,
      require_first_turn_success: REQUIRE_FIRST_TURN_SUCCESS,
      require_all_harness_installs: REQUIRE_ALL_HARNESS_INSTALLS,
      skip_managed_binary_reset: SKIP_MANAGED_BINARY_RESET,
      managed_provider_ids: REQUIRE_ALL_HARNESS_INSTALLS ? resolveManagedProviderInstallIds() : [],
      perf_budgets: perfBudgets,
      scenario_filter: Array.from(scenarioFilter.values()),
      fixture,
    });

    if (!scenarioEnabled("remote-new-sandbox", ["remote", "sandbox"])) {
      const detail = "scenario filter excluded remote-new-sandbox";
      contractRecorder.recordAssertion("scenario_filter", "skip", detail);
      finalizeReport({ result: "skipped", reason: detail });
      this.skip();
    }

    if (!fixture.ready) {
      const detail = fixture.preflightMessage;
      if (fixture.strictRequired) {
        contractRecorder.recordAssertion("fixture_preflight", "fail", detail);
        finalizeReport({ result: "failed", reason: detail, error: detail });
        throw new Error(detail);
      }
      contractRecorder.recordAssertion("fixture_preflight", "skip", detail);
      finalizeReport({ result: "skipped", reason: detail });
      this.skip();
    }

    try {
      contractRecorder.recordAssertion("fixture_preflight", "pass", "resolved remote sandbox fixture contract");

      resetRemoteBootstrapState();
      const beforeState = managedBinaryState();
      contractRecorder.recordArtifact("managed_binary_state_before_connect", { state: beforeState });
      if (!SKIP_MANAGED_BINARY_RESET && beforeState !== "missing") {
        throw new Error(`expected managed binary to be missing before connect, got '${beforeState}'`);
      }

      await navigateToTauriUrl(`tauri://localhost/workspace-setup?remoteContainerContract=${Date.now()}`);

      const sandboxCliProbe = remoteSsh(
        "if { [ -n \"${CTX_HARNESS_SANDBOX_CLI_PATH:-}\" ] && [ -x \"${CTX_HARNESS_SANDBOX_CLI_PATH}\" ]; } || command -v nerdctl >/dev/null 2>&1; then echo yes; else echo no; fi",
        {
          auth: remoteAuthMode,
          label: "sandbox-cli-probe",
        },
      ).trim();
      contractRecorder.recordArtifact("sandbox_cli_probe", { result: sandboxCliProbe });
      contractRecorder.recordAssertion(
        "sandbox_cli_probe",
        "pass",
        sandboxCliProbe === "yes"
          ? "remote sandbox substrate already present before bootstrap"
          : "remote sandbox substrate absent before bootstrap; bootstrap path must provision it",
      );

      const remoteDest = `${remoteBase}/new-sandbox`;
      workspaceId = await runWizardScenario({
        location: "remote",
        remoteHost: fixture.wizardHostInput,
        remotePort: fixture.port,
        remoteDataDir,
        container: "sandbox",
        network: "full",
        harnessDownloads: "skip",
        source: { kind: "new", destPath: remoteDest, workspaceName: "remote-contract" },
        setupHook: "",
        mergeQueue: { kind: "skip" },
      });

      const workspace = await getWorkspace(workspaceId);
      rootPath = String(workspace.root_path || "");
      contractRecorder.recordArtifact("workspace", workspace);
      const expectedRootPrefix = expectedRemoteRootPrefix({
        remoteBase,
        remoteDataDir,
        container: "sandbox",
        sourceKind: "new",
      });
      if (!rootPath.startsWith(expectedRootPrefix)) {
        throw new Error(`expected remote root under ${expectedRootPrefix}, got ${rootPath}`);
      }

      const afterState = managedBinaryState();
      contractRecorder.recordArtifact("managed_binary_state_after_connect", { state: afterState });
      if (afterState !== "present") {
        throw new Error(`expected managed binary at ${REMOTE_CTX_BIN}, got '${afterState}'`);
      }
      const helpOutput = remoteSsh(`if [ -x ${REMOTE_CTX_BIN} ]; then ${REMOTE_CTX_BIN} --help; else echo missing; fi`, {
        auth: remoteAuthMode,
        label: "managed-binary-help",
      });
      contractRecorder.recordArtifact("managed_binary_help_output", helpOutput);
      if (!helpOutput || helpOutput === "missing" || !helpOutput.includes("Usage: ctx")) {
        throw new Error(`expected installed managed binary to execute and print usage, got '${helpOutput}'`);
      }

      await assertDesktopSshConnection("after_connect");
      await assertNoDaemonOverlayFor(20_000);
      contractRecorder.recordAssertion("workspace_launch", "pass", "remote workspace launched without daemon overlay");
      const daemonHealth = await safeDaemonJson("GET", "/api/health");
      contractRecorder.recordArtifact("daemon_health_post_bootstrap", daemonHealth);
      if (daemonHealth.status !== 200) {
        throw new Error(`expected /api/health 200 after remote sandbox launch, got ${daemonHealth.status}`);
      }

      try {
        provider = await ensureCodexOpenRouterWorkspaceReady(workspaceId, {
          installTarget: "container",
          endpointName: `remote-container-openrouter-${runId}`,
        });
        contractRecorder.recordArtifact("provider_verify_payload", provider.verifyPayload || null);
        firstTurn = await runDeterministicFirstTurnOutcome(
          workspaceId,
          {
            providerId: provider.providerId,
            modelId: provider.modelId,
            prompt: "hello",
            timeoutMs: 180000,
          },
        );
        firstTurn = { attempted: true, ...firstTurn, provider };
        if (REQUIRE_FIRST_TURN_SUCCESS && firstTurn.status !== "success") {
          const failure = new Error(`expected first turn success, got ${JSON.stringify(firstTurn)}`);
          failure.firstTurnOutcome = firstTurn;
          throw failure;
        }
      } catch (error) {
        firstTurn = error?.firstTurnOutcome || {
          attempted: true,
          status: "failed",
          error: String(error),
        };
        if (REQUIRE_FIRST_TURN_SUCCESS) {
          throw error;
        }
      }

      contractRecorder.recordArtifact("first_turn", firstTurn);
      contractRecorder.recordAssertion(
        "first_turn",
        firstTurn.status === "success" ? "pass" : "warn",
        firstTurn.status === "success" ? "remote sandbox first turn succeeded" : JSON.stringify(firstTurn),
      );

      if (REQUIRE_ALL_HARNESS_INSTALLS) {
        const managedProviderInstalls = await installManagedProvidersAndAssertInstalled("container", {
          recorder: contractRecorder,
          artifactPrefix: "sandbox_acceptance",
        });
        contractRecorder.recordArtifact("sandbox_acceptance_managed_provider_installs", managedProviderInstalls);
      }

      const postLaunchState = collectRemoteRuntimeState(remoteAuthMode, "remote_state_after_initial_launch");
      await assertProviderInstalledNoInstallRunning("container", "post_launch");

      const disconnectResp = await tauriInvoke("desktop_disconnect", {});
      if (disconnectResp.error) {
        throw new Error(`desktop_disconnect failed: ${disconnectResp.error}`);
      }

      const warmReconnect = await recordTimedOperation("warm_reconnect", async () => {
        return await connectSshWithPolling(reconnectReq());
      });
      const reconnectResp = warmReconnect.value;
      contractRecorder.recordArtifact("reconnect_response", reconnectResp);
      if (reconnectResp.error) {
        throw new Error(`desktop reconnect failed: ${reconnectResp.error}`);
      }
      assertElapsedWithinBudget("warm_reconnect_budget", warmReconnect, perfBudgets.warm_connect_ms);
      await assertDesktopSshConnection("after_reconnect");

      const postReconnectHealth = await safeDaemonJson("GET", "/api/health");
      contractRecorder.recordArtifact("daemon_health_post_warm_reconnect", postReconnectHealth);
      if (postReconnectHealth.status !== 200) {
        throw new Error(`expected /api/health 200 after warm reconnect, got ${postReconnectHealth.status}`);
      }

      const postWarmReconnectState = collectRemoteRuntimeState(remoteAuthMode, "remote_state_after_warm_reconnect");
      assertSameBinaryFingerprint(
        postLaunchState.binary,
        postWarmReconnectState.binary,
        "warm_reconnect_binary_stable",
      );
      recordDaemonProcessObservation(
        postLaunchState.daemon,
        postWarmReconnectState.daemon,
        "warm_reconnect_daemon_observation",
      );

      await assertNoDaemonOverlayFor(10_000);
      contractRecorder.recordArtifact("workspace_route_post_warm_reconnect", await collectWorkspaceRouteDiagnostics());

      const warmWorkspace = await getWorkspace(workspaceId);
      contractRecorder.recordArtifact("workspace_post_warm_reconnect", warmWorkspace);
      if (String(warmWorkspace.root_path || "") !== rootPath) {
        throw new Error(`workspace root changed after warm reconnect: expected ${rootPath}, got ${String(warmWorkspace.root_path || "")}`);
      }

      await assertWarmWorkspaceUsability({
        workspaceId,
        installTarget: "container",
        endpointName: `remote-container-openrouter-warm-${runId}-${Date.now()}`,
        perfLabel: "warm_reconnect",
      });
      contractRecorder.recordAssertion(
        "warm_reconnect",
        "pass",
        "warm reconnect reused the managed binary, existing daemon, and installed container provider",
      );

      const disconnectResp2 = await tauriInvoke("desktop_disconnect", {});
      contractRecorder.recordArtifact("disconnect_response_second", disconnectResp2);
      if (disconnectResp2.error) {
        throw new Error(`second desktop_disconnect failed: ${disconnectResp2.error}`);
      }

      const secondWarmReconnect = await recordTimedOperation("second_warm_reconnect", async () => {
        return await connectSshWithPolling(reconnectReq());
      });
      const reconnectResp2 = secondWarmReconnect.value;
      contractRecorder.recordArtifact("reconnect_response_second", reconnectResp2);
      if (reconnectResp2.error) {
        throw new Error(`second desktop reconnect failed: ${reconnectResp2.error}`);
      }
      assertElapsedWithinBudget("second_warm_reconnect_budget", secondWarmReconnect, perfBudgets.warm_connect_ms);
      await assertDesktopSshConnection("after_reconnect_again");
      await assertNoDaemonOverlayFor(10_000);
      const secondReconnectHealth = await safeDaemonJson("GET", "/api/health");
      contractRecorder.recordArtifact("daemon_health_post_second_warm_reconnect", secondReconnectHealth);
      if (secondReconnectHealth.status !== 200) {
        throw new Error(`expected /api/health 200 after second warm reconnect, got ${secondReconnectHealth.status}`);
      }
      const workspaceAfterSecondReconnect = await getWorkspace(workspaceId);
      contractRecorder.recordArtifact("workspace_post_second_warm_reconnect", workspaceAfterSecondReconnect);
      if (String(workspaceAfterSecondReconnect.root_path || "") !== rootPath) {
        throw new Error(
          `workspace root changed after second warm reconnect: expected ${rootPath}, got ${String(workspaceAfterSecondReconnect.root_path || "")}`,
        );
      }
      contractRecorder.recordAssertion(
        "warm_reconnect_again",
        "pass",
        "second reconnect preserved desktop SSH state and remote sandbox workspace identity",
      );
    } catch (error) {
      finalResult = "failed";
      finalReason = "remote sandbox contract failed";
      finalError = String(error);
      contractRecorder.recordAssertion("contract", "fail", finalError);
      await collectFailureArtifacts("failure");
      throw error;
    } finally {
      finalizeReport({
        result: finalResult,
        reason: finalReason,
        error: finalError,
        extras: {
          workspace_id: workspaceId || null,
          root_path: rootPath || null,
          perf_budgets: perfBudgets,
          provider_config: provider,
          first_turn: firstTurn,
        },
      });
    }
  });
});
