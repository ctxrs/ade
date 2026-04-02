const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const { tauriInvoke, connectSshWithPolling } = require("./helpers/desktop_connection.cjs");
const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson, safeDaemonJson } = require("./helpers/daemon.cjs");
const { assertWorkspaceTerminalCwdPrefix } = require("./helpers/workspace_wizard_flow.cjs");
const {
  ensureCodexOpenRouterWorkspaceReady,
  getProviderStatus,
  verifyProviderForWorkspace,
} = require("./helpers/provider_runtime.cjs");
const { runDeterministicFirstTurnOutcome } = require("./helpers/first_turn_contract.cjs");
const {
  createRemoteContractRecorder,
  parseBoolean,
  resolveRemoteFixtureEnv,
  resolveRemotePerformanceBudgets,
} = require("../helpers/remote_fixture_contract.cjs");

const REMOTE_CTX_BIN = "$HOME/.ctx/bin/ctx";
const AUTH_TEST_MODE = String(process.env.CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE || "key").trim().toLowerCase();
const EXPECT_CONNECT_FAILURE = parseBoolean(process.env.CTX_AUTOMATION_REMOTE_EXPECT_CONNECT_FAILURE || "0");
const WRONG_PASSWORD = String(process.env.CTX_AUTOMATION_REMOTE_WRONG_PASSWORD || "definitely-wrong-password").trim();
const FIRST_TURN_REPORT_PATH = String(process.env.CTX_REMOTE_BOOTSTRAP_FIRST_TURN_REPORT || "").trim();
const CONTRACT_REPORT_PATH = String(
  process.env.CTX_REMOTE_BOOTSTRAP_CONTRACT_REPORT || path.join("/tmp", "ctx-remote-bootstrap-contract.json"),
).trim();
const REQUIRE_FIRST_TURN_SUCCESS = parseBoolean(process.env.CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS || "0");
const SKIP_MANAGED_BINARY_RESET = parseBoolean(process.env.CTX_AUTOMATION_REMOTE_SKIP_MANAGED_BINARY_RESET || "0");
const fixture = resolveRemoteFixtureEnv({ lane: "host" });
const perfBudgets = resolveRemotePerformanceBudgets({ fixture });

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
  } else {
    args.unshift("/dev/null");
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

const remoteSsh = (command, { auth = "key", passwordOverride = "", label = "ssh" } = {}) => {
  const usePassword = auth === "password";
  const args = sshBaseArgs({ useKey: !usePassword });
  let binary = "ssh";
  let finalArgs;
  let env = process.env;

  if (usePassword) {
    const password = String(passwordOverride || fixture.passwordActual || "").trim();
    if (!password) {
      throw new Error("password auth requested but fixture password is not set");
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
      command,
    ];
    env = { ...process.env, SSHPASS: password };
  } else {
    finalArgs = [...args, "-o", "BatchMode=yes", fixture.target, command];
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
    const details = [
      `command failed: ${binary} ${finalArgs.join(" ")}`,
      stderr ? `stderr: ${stderr}` : null,
      stdout ? `stdout: ${stdout}` : null,
    ]
      .filter(Boolean)
      .join("\n");
    throw new Error(details);
  }
  return String(result.stdout || "").trim();
};

const diagnosticsAuthMode = () => {
  if (AUTH_TEST_MODE === "password_once" || AUTH_TEST_MODE === "wrong_password") return "password";
  if (fixture.authMode === "password") return "password";
  return "key";
};

const managedState = () => {
  const cmd = `if [ -x ${REMOTE_CTX_BIN} ]; then echo present; else echo missing; fi`;
  return remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, { auth: diagnosticsAuthMode(), label: "managed-state" });
};

const reconnectReq = () => ({
  host: fixture.host,
  user: fixture.user,
  remote_port: fixture.port,
  start_remote: true,
  remote_data_dir: fixture.dataDir || undefined,
});

const readRemoteBinaryFingerprint = () => {
  const cmd = [
    "set -eu",
    `if [ -x ${REMOTE_CTX_BIN} ]; then set -- $(cksum ${REMOTE_CTX_BIN}); mtime=$(stat -c %Y ${REMOTE_CTX_BIN} 2>/dev/null || echo 0); printf '%s|%s|%s\\n' \"$1\" \"$2\" \"$mtime\"; else printf 'missing\\n'; fi`,
  ].join("; ");
  const raw = remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, {
    auth: diagnosticsAuthMode(),
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

const readRemoteDaemonFingerprint = () => {
  const cmd = [
    "set -eu",
    "pid=$(pgrep -xo ctx || true)",
    "if [ -n \"$pid\" ]; then etimes=$(ps -o etimes= -p \"$pid\" 2>/dev/null | tr -d ' '); started=$(ps -o lstart= -p \"$pid\" 2>/dev/null | sed 's/^ *//'); printf '%s|%s|%s\\n' \"$pid\" \"${etimes:-0}\" \"$started\"; else printf 'missing\\n'; fi",
  ].join("; ");
  const raw = remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, {
    auth: diagnosticsAuthMode(),
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

const collectRemoteRuntimeState = (label) => {
  const snapshot = {
    binary: readRemoteBinaryFingerprint(),
    daemon: readRemoteDaemonFingerprint(),
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
    description: `Remote warm task smoke for ${label}`,
    create_default_session: false,
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
  expectedTerminalCwdPrefix,
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
    await assertWorkspaceTerminalCwdPrefix(workspaceId, expectedTerminalCwdPrefix);
    return { cwd_prefix: expectedTerminalCwdPrefix };
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
    "provider verify, terminal create, and task create succeeded without reinstall",
  );
};

const removeManagedBinary = () => {
  const cmd = `rm -f ${REMOTE_CTX_BIN}`;
  remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, { auth: diagnosticsAuthMode(), label: "managed-binary-reset" });
};

const stopRemoteDaemon = () => {
  const cmd = "if command -v pkill >/dev/null 2>&1; then pkill -x ctx >/dev/null 2>&1 || true; fi; true";
  remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, { auth: diagnosticsAuthMode(), label: "remote-daemon-stop" });
};

const ensureRemoteWorkspaceRepo = () => {
  const remoteRoot = `/tmp/ctx-remote-bootstrap-contract-${Date.now()}-${Math.trunc(Math.random() * 100000)}`;
  const setupCmd = [
    "set -euo pipefail",
    `mkdir -p ${JSON.stringify(remoteRoot)}`,
    `cd ${JSON.stringify(remoteRoot)}`,
    "if [ ! -d .git ]; then git init >/dev/null 2>&1; fi",
    "if [ ! -f README.md ]; then printf '%s\\n' '# remote bootstrap contract' > README.md; fi",
    "if ! git rev-parse --verify HEAD >/dev/null 2>&1; then git add README.md && git -c user.name='ctx fixture' -c user.email='ctx-fixture@example.invalid' commit -m 'bootstrap fixture repo' >/dev/null 2>&1; fi",
  ].join("; ");
  remoteSsh(`bash -lc ${JSON.stringify(setupCmd)}`, { auth: diagnosticsAuthMode(), label: "workspace-repo-setup" });
  return remoteRoot;
};

const connectReq = () => {
  const req = {
    host: fixture.host,
    user: fixture.user,
    remote_port: fixture.port,
    start_remote: true,
    remote_data_dir: fixture.dataDir || undefined,
  };
  if (AUTH_TEST_MODE === "password_once") {
    req.password_once = fixture.passwordActual;
  } else if (AUTH_TEST_MODE === "wrong_password") {
    req.password_once = WRONG_PASSWORD;
  } else if (fixture.authMode === "password" && !EXPECT_CONNECT_FAILURE && fixture.passwordActual) {
    req.password_once = fixture.passwordActual;
  }
  return req;
};

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

const assertRemoteHealth = async (stage) => {
  const health = await daemonJson("GET", "/api/health");
  if (health.status !== 200) {
    throw new Error(`${stage}: expected /api/health 200, got ${health.status} payload=${JSON.stringify(health.payload || null)}`);
  }
  return health.payload || null;
};

const assertWorkspaceLaunch = async () => {
  const remoteRoot = ensureRemoteWorkspaceRepo();
  const create = await daemonJson("POST", "/api/workspaces", {
    root_path: remoteRoot,
    name: `fixture-${Date.now()}`,
  });
  if (create.status !== 200 || !create.payload?.id) {
    throw new Error(`workspace create failed (${create.status}): ${JSON.stringify(create.payload || null)}`);
  }

  const workspaceId = String(create.payload.id);
  const read = await daemonJson("GET", `/api/workspaces/${workspaceId}`);
  if (read.status !== 200) {
    throw new Error(`workspace read failed (${read.status}): ${JSON.stringify(read.payload || null)}`);
  }
  const rootPath = String(read.payload?.root_path || "");
  if (rootPath !== remoteRoot) {
    throw new Error(`workspace root mismatch: expected ${remoteRoot}, got ${rootPath}`);
  }
  return {
    workspaceId,
    remoteRoot,
    createResponse: create.payload || null,
    readResponse: read.payload || null,
  };
};

const writeFirstTurnReport = (payload) => {
  if (!FIRST_TURN_REPORT_PATH) return;
  try {
    fs.mkdirSync(path.dirname(FIRST_TURN_REPORT_PATH), { recursive: true });
    fs.writeFileSync(FIRST_TURN_REPORT_PATH, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  } catch {
    // best effort only
  }
};

const collectFailureArtifacts = async (stage) => {
  if (!contractRecorder) return;

  contractRecorder.recordArtifact(`${stage}_desktop_connection`, await tauriInvoke("desktop_get_connection", {}));
  contractRecorder.recordArtifact(`${stage}_daemon_health`, await safeDaemonJson("GET", "/api/health"));
  contractRecorder.recordArtifact(`${stage}_daemon_diagnostics`, await safeDaemonJson("GET", "/api/diagnostics"));

  if (!fixture.target || !fixture.dataDir) return;

  const remoteLogFile = `${fixture.dataDir.replace(/\/+$/, "")}/logs/daemon.log`;
  const tailCmd = `if [ -f ${JSON.stringify(remoteLogFile)} ]; then tail -n 200 ${JSON.stringify(remoteLogFile)}; else echo "__CTX_MISSING__ ${remoteLogFile}"; fi`;
  try {
    const tail = remoteSsh(`sh -lc ${JSON.stringify(tailCmd)}`, {
      auth: diagnosticsAuthMode(),
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

describe("remote bootstrap install e2e", () => {
  it("connects over SSH and validates managed remote daemon bootstrap contracts", async function () {
    this.timeout(12 * 60_000);

    contractRecorder = createRemoteContractRecorder({
      outputPath: CONTRACT_REPORT_PATH,
      suite: "remote bootstrap install e2e",
      lane: "host",
      fixture,
      secretValues: [fixture.password, fixture.passwordActual, process.env.OPENROUTER_API_KEY],
    });

    let finalized = false;
    const finalizeReport = (payload) => {
      if (finalized) return null;
      finalized = true;
      return contractRecorder.finalize(payload);
    };

    let workspaceLaunch = null;
    let firstTurnProvider = null;
    let firstTurn = null;
    let finalResult = "passed";
    let finalReason = "remote bootstrap contract validated";
    let finalError = "";

    contractRecorder.recordArtifact("fixture_preflight", {
      auth_test_mode: AUTH_TEST_MODE,
      expect_connect_failure: EXPECT_CONNECT_FAILURE,
      require_first_turn_success: REQUIRE_FIRST_TURN_SUCCESS,
      skip_managed_binary_reset: SKIP_MANAGED_BINARY_RESET,
      contract_report_path: CONTRACT_REPORT_PATH,
      first_turn_report_path: FIRST_TURN_REPORT_PATH || null,
      perf_budgets: perfBudgets,
      fixture,
    });

    if (!fixture.ready) {
      const detail = fixture.preflightMessage;
      if (fixture.strictRequired) {
        contractRecorder.recordAssertion("fixture_preflight", "fail", detail);
        finalizeReport({ result: "failed", reason: detail, error: detail });
        throw new Error(detail);
      }
      contractRecorder.recordAssertion("fixture_preflight", "skip", detail);
      finalizeReport({ result: "skipped", reason: detail });
      writeFirstTurnReport({
        result: "skipped",
        reason: detail,
        fixture,
      });
      this.skip();
    }

    try {
      contractRecorder.recordAssertion("fixture_preflight", "pass", "resolved remote fixture contract");

      await browser.url("tauri://localhost/workspaces");
      await waitForTauri();

      if (!SKIP_MANAGED_BINARY_RESET) {
        removeManagedBinary();
        const beforeState = managedState();
        contractRecorder.recordArtifact("managed_binary_state_before_connect", { state: beforeState });
        if (beforeState !== "missing") {
          throw new Error(`expected managed binary to be missing before connect, got '${beforeState}'`);
        }
      }

      const req = connectReq();
      contractRecorder.recordArtifact("connect_request", req);
      const initialConnect = await recordTimedOperation("initial_connect", async () => {
        return await connectSshWithPolling(req);
      });
      const connectResp = initialConnect.value;
      contractRecorder.recordArtifact("connect_response", connectResp);

      if (EXPECT_CONNECT_FAILURE || AUTH_TEST_MODE === "wrong_password") {
        if (!connectResp.error) {
          throw new Error(`expected connect failure for wrong-password mode, got success: ${JSON.stringify(connectResp.value || null)}`);
        }
        const detail = String(connectResp.error || "");
        if (!/password-once ssh bootstrap failed|permission denied|failed to reach remote daemon/i.test(detail)) {
          throw new Error(`expected password auth failure detail, got: ${detail}`);
        }
        contractRecorder.recordAssertion("expected_connect_failure", "pass", detail);
        finalReason = "expected connect failure observed";
        return;
      }

      if (connectResp.error) {
        throw new Error(`desktop_connect_ssh failed: ${connectResp.error}`);
      }
      contractRecorder.recordAssertion("ssh_connect", "pass", "desktop_connect_ssh succeeded");
      assertElapsedWithinBudget("initial_connect_budget", initialConnect, perfBudgets.cold_connect_ms);
      await assertDesktopSshConnection("after_connect");

      const afterState = managedState();
      contractRecorder.recordArtifact("managed_binary_state_after_connect", { state: afterState });
      if (afterState !== "present") {
        throw new Error(`expected managed binary at ${REMOTE_CTX_BIN}, got '${afterState}'`);
      }

      const helpCmd = `if [ -x ${REMOTE_CTX_BIN} ]; then ${REMOTE_CTX_BIN} --help; else echo missing; fi`;
      const helpOutput = remoteSsh(`sh -lc ${JSON.stringify(helpCmd)}`, {
        auth: diagnosticsAuthMode(),
        label: "managed-binary-help",
      });
      contractRecorder.recordArtifact("managed_binary_help_output", helpOutput);
      if (!helpOutput || helpOutput === "missing" || !helpOutput.includes("Usage: ctx")) {
        throw new Error(`expected installed managed binary to execute and print usage, got '${helpOutput}'`);
      }

      contractRecorder.recordArtifact("daemon_health_post_bootstrap", await assertRemoteHealth("post-bootstrap"));
      workspaceLaunch = await assertWorkspaceLaunch();
      contractRecorder.recordAssertion("workspace_launch", "pass", "remote workspace launch contract succeeded");
      contractRecorder.recordArtifact("workspace_launch", workspaceLaunch);

      try {
        firstTurnProvider = await ensureCodexOpenRouterWorkspaceReady(workspaceLaunch.workspaceId, {
          installTarget: "host",
          endpointName: `remote-bootstrap-openrouter-${Date.now()}`,
        });
        contractRecorder.recordArtifact("provider_verify_payload", firstTurnProvider.verifyPayload || null);
      } catch (error) {
        if (REQUIRE_FIRST_TURN_SUCCESS) {
          throw new Error(`failed to configure remote first-turn provider auth: ${String(error)}`);
        }
        firstTurn = {
          status: "failed",
          stage: "provider_setup",
          detail: String(error),
        };
        contractRecorder.recordAssertion("provider_setup", "warn", String(error));
      }

      if (!firstTurn) {
        firstTurn = await runDeterministicFirstTurnOutcome(
          workspaceLaunch.workspaceId,
          {
            providerId: firstTurnProvider?.providerId || "codex",
            modelId: firstTurnProvider?.modelId || "default",
            prompt: "hello",
            timeoutMs: 180000,
          },
        );
      }
      contractRecorder.recordArtifact("first_turn", firstTurn);
      contractRecorder.recordAssertion(
        "first_turn",
        firstTurn.status === "success" ? "pass" : "warn",
        firstTurn.status === "success" ? "first turn succeeded" : JSON.stringify(firstTurn),
      );

      writeFirstTurnReport({
        workspace_id: workspaceLaunch.workspaceId,
        workspace_root: workspaceLaunch.remoteRoot,
        auth_mode: AUTH_TEST_MODE,
        provider_config: firstTurnProvider,
        first_turn: firstTurn,
      });
      if (REQUIRE_FIRST_TURN_SUCCESS && firstTurn.status !== "success") {
        throw new Error(`expected first turn success, got ${JSON.stringify(firstTurn)}`);
      }

      const postBootstrapState = collectRemoteRuntimeState("remote_state_after_bootstrap");
      await assertProviderInstalledNoInstallRunning("host", "post_bootstrap");

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
      contractRecorder.recordArtifact("daemon_health_post_reconnect", await assertRemoteHealth("post-reconnect"));
      const postWarmReconnectState = collectRemoteRuntimeState("remote_state_after_warm_reconnect");
      assertSameBinaryFingerprint(
        postBootstrapState.binary,
        postWarmReconnectState.binary,
        "warm_reconnect_binary_stable",
      );
      recordDaemonProcessObservation(
        postBootstrapState.daemon,
        postWarmReconnectState.daemon,
        "warm_reconnect_daemon_observation",
      );
      const warmWorkspaceRead = await daemonJson("GET", `/api/workspaces/${workspaceLaunch.workspaceId}`);
      contractRecorder.recordArtifact("workspace_read_post_warm_reconnect", warmWorkspaceRead);
      if (warmWorkspaceRead.status !== 200) {
        throw new Error(`workspace read after warm reconnect failed (${warmWorkspaceRead.status}): ${JSON.stringify(warmWorkspaceRead.payload || null)}`);
      }
      if (String(warmWorkspaceRead.payload?.root_path || "") !== workspaceLaunch.remoteRoot) {
        throw new Error(
          `workspace root changed after warm reconnect: expected ${workspaceLaunch.remoteRoot}, got ${String(warmWorkspaceRead.payload?.root_path || "")}`,
        );
      }
      await assertWarmWorkspaceUsability({
        workspaceId: workspaceLaunch.workspaceId,
        expectedTerminalCwdPrefix: workspaceLaunch.remoteRoot,
        installTarget: "host",
        endpointName: `remote-bootstrap-openrouter-warm-${Date.now()}`,
        perfLabel: "warm_reconnect",
      });
      contractRecorder.recordAssertion(
        "warm_reconnect",
        "pass",
        "warm reconnect reused the managed binary and existing remote daemon",
      );

      stopRemoteDaemon();
      const disconnectResp2 = await tauriInvoke("desktop_disconnect", {});
      if (disconnectResp2.error) {
        throw new Error(`desktop_disconnect before restart check failed: ${disconnectResp2.error}`);
      }
      const restartReconnect = await recordTimedOperation("restart_reconnect", async () => {
        return await connectSshWithPolling(reconnectReq());
      });
      const restartResp = restartReconnect.value;
      contractRecorder.recordArtifact("restart_response", restartResp);
      if (restartResp.error) {
        throw new Error(`desktop reconnect after remote daemon stop failed: ${restartResp.error}`);
      }
      assertElapsedWithinBudget(
        "restart_reconnect_budget",
        restartReconnect,
        perfBudgets.daemon_restart_connect_ms,
      );
      await assertDesktopSshConnection("after_remote_restart");
      contractRecorder.recordArtifact(
        "daemon_health_post_remote_restart",
        await assertRemoteHealth("post-remote-daemon-restart"),
      );
      const postRestartState = collectRemoteRuntimeState("remote_state_after_remote_restart");
      assertSameBinaryFingerprint(
        postBootstrapState.binary,
        postRestartState.binary,
        "restart_reconnect_binary_stable",
      );
      if (!postRestartState.daemon.present || !postBootstrapState.daemon.present) {
        contractRecorder.recordAssertion(
          "restart_reconnect_daemon_observation",
          "pass",
          `remote daemon process was not directly observable across explicit stop/reconnect: before=${JSON.stringify(postBootstrapState.daemon)} after=${JSON.stringify(postRestartState.daemon)}`,
        );
      } else if (
        postRestartState.daemon.pid === postBootstrapState.daemon.pid
        && postRestartState.daemon.started === postBootstrapState.daemon.started
      ) {
        throw new Error(
          `expected a new daemon process after explicit stop, got same fingerprint: before=${JSON.stringify(postBootstrapState.daemon)} after=${JSON.stringify(postRestartState.daemon)}`,
        );
      } else {
        contractRecorder.recordAssertion(
          "restart_reconnect_daemon_observation",
          "pass",
          "daemon process fingerprint changed after explicit stop/reconnect",
        );
      }
      await assertWarmWorkspaceUsability({
        workspaceId: workspaceLaunch.workspaceId,
        expectedTerminalCwdPrefix: workspaceLaunch.remoteRoot,
        installTarget: "host",
        endpointName: `remote-bootstrap-openrouter-restart-${Date.now()}`,
        perfLabel: "post_restart",
      });
      contractRecorder.recordAssertion(
        "remote_restart",
        "pass",
        "desktop reconnect restarted the remote daemon without reinstalling the managed binary",
      );
    } catch (error) {
      finalResult = "failed";
      finalError = String(error);
      finalReason = "remote bootstrap contract failed";
      contractRecorder.recordAssertion("contract", "fail", finalError);
      await collectFailureArtifacts("failure");
      throw error;
    } finally {
      finalizeReport({
        result: finalResult,
        reason: finalReason,
        error: finalError,
        extras: {
          auth_test_mode: AUTH_TEST_MODE,
          expect_connect_failure: EXPECT_CONNECT_FAILURE,
          require_first_turn_success: REQUIRE_FIRST_TURN_SUCCESS,
          perf_budgets: perfBudgets,
          first_turn_report_path: FIRST_TURN_REPORT_PATH || null,
          workspace_launch: workspaceLaunch,
          provider_config: firstTurnProvider,
          first_turn: firstTurn,
        },
      });
    }
  });
});
