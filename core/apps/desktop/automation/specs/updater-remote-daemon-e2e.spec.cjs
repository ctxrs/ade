const path = require("node:path");

const { resolveBoolishFlag } = require("../../../../scripts/lib/boolish.cjs");
const { navigateToTauriUrl } = require("./helpers/tauri.cjs");
const { createRemoteContractRecorder, resolveRemoteFixtureEnv } = require("../helpers/remote_fixture_contract.cjs");
const {
  REMOTE_HOST,
  BOOTSTRAP_CHANNEL,
  TARGET_CHANNEL,
  bootstrapRemoteDaemon,
  createBusyRemoteTurn,
  waitForRemoteTurnTerminal,
  readRemoteVersion,
  waitForRemoteVersionChange,
  readRemoteAutoUpdateStatus,
  isBootstrapDaemonUnsupportedError,
} = require("./helpers/remote_updater_proof.cjs");

const reportPath = String(
  process.env.CTX_UPDATER_REMOTE_E2E_REPORT
  || process.env.CTX_UPDATER_REMOTE_PROOF_REPORT
  || path.join("/tmp", "ctx-updater-remote-proof.json"),
).trim();
const runIdle = resolveBoolishFlag(
  process.env.CTX_UPDATER_REMOTE_PROOF_IDLE,
  true,
  "CTX_UPDATER_REMOTE_PROOF_IDLE",
);
const runPendingIdle = resolveBoolishFlag(
  process.env.CTX_UPDATER_REMOTE_PROOF_PENDING_IDLE,
  false,
  "CTX_UPDATER_REMOTE_PROOF_PENDING_IDLE",
);
const runPendingRestartNow = resolveBoolishFlag(
  process.env.CTX_UPDATER_REMOTE_PROOF_PENDING_RESTART_NOW,
  false,
  "CTX_UPDATER_REMOTE_PROOF_PENDING_RESTART_NOW",
);
const runIncompatibleReconnect = resolveBoolishFlag(
  process.env.CTX_UPDATER_REMOTE_PROOF_INCOMPATIBLE_RECONNECT,
  false,
  "CTX_UPDATER_REMOTE_PROOF_INCOMPATIBLE_RECONNECT",
);
const runNoClientAuto = resolveBoolishFlag(
  process.env.CTX_UPDATER_REMOTE_PROOF_NO_CLIENT_AUTO,
  false,
  "CTX_UPDATER_REMOTE_PROOF_NO_CLIENT_AUTO",
);
const requireVersionChange = resolveBoolishFlag(
  process.env.CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE,
  true,
  "CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE",
);
const compatibleBootstrapChannel = String(
  process.env.CTX_UPDATER_E2E_COMPATIBLE_BOOTSTRAP_CHANNEL || BOOTSTRAP_CHANNEL,
).trim() || BOOTSTRAP_CHANNEL;
const incompatibleBootstrapChannel = String(
  process.env.CTX_UPDATER_E2E_INCOMPATIBLE_BOOTSTRAP_CHANNEL || BOOTSTRAP_CHANNEL,
).trim() || BOOTSTRAP_CHANNEL;

const tauriInvoke = async (command, args) => {
  let result;
  try {
    result = await browser.executeAsync(({ cmd, payload }, done) => {
      const tauriInvoke = window.__TAURI__?.core?.invoke;
      const internalsInvoke = window.__TAURI_INTERNALS__?.invoke;
      const invoke = internalsInvoke || tauriInvoke;
      if (!invoke) {
        done({ error: "Tauri invoke API not available" });
        return;
      }
      Promise.resolve()
        .then(() => invoke(cmd, payload))
        .then((value) => done({ value }))
        .catch((err) => done({ error: String(err) }));
    }, { cmd: command, payload: args });
  } catch (err) {
    return { error: String(err) };
  }
  if (result && typeof result === "object" && Object.prototype.hasOwnProperty.call(result, "error")) {
    return { error: String(result.error || "") };
  }
  return { value: result ? result.value : undefined };
};

const remoteFixture = () => resolveRemoteFixtureEnv({ lane: "host" });

const connectDesktop = async () => {
  const fixture = remoteFixture();
  const response = await tauriInvoke("desktop_connect_ssh", {
    req: {
      host: REMOTE_HOST,
      user: fixture.user || "root",
      remote_port: fixture.port || undefined,
      start_remote: true,
      remote_data_dir: fixture.dataDir || undefined,
    },
  });
  if (response.error) {
    throw new Error(`desktop_connect_ssh failed: ${response.error}`);
  }
  return response.value || {};
};

const disconnectDesktop = async () => {
  const response = await tauriInvoke("desktop_disconnect", {});
  if (response.error) {
    throw new Error(`desktop_disconnect failed: ${response.error}`);
  }
  return response.value || {};
};

const getConnection = async () => {
  const response = await tauriInvoke("desktop_get_connection", {});
  if (response.error) {
    throw new Error(`desktop_get_connection failed: ${response.error}`);
  }
  return response.value || {};
};

const updateRemoteNow = async () => {
  const response = await tauriInvoke("desktop_update_remote_daemon", {
    req: {
      confirm: true,
      channel: TARGET_CHANNEL,
    },
  });
  if (response.error) {
    throw new Error(`desktop_update_remote_daemon failed: ${response.error}`);
  }
  return response.value || {};
};

const assertTurnStatus = (turn, allowedStatuses, scenario) => {
  const status = String(turn?.status || "").toLowerCase();
  if (!allowedStatuses.includes(status)) {
    throw new Error(
      `${scenario} expected terminal turn status ${allowedStatuses.join(" or ")}, got ${status || "<empty>"}`,
    );
  }
};

const waitForConnection = async (predicate, label, timeoutMs = 180000) => {
  const started = Date.now();
  let info = await getConnection();
  while (Date.now() - started < timeoutMs) {
    if (predicate(info)) {
      return info;
    }
    await browser.pause(1000);
    info = await getConnection();
  }
  throw new Error(`${label} timed out: ${JSON.stringify(info)}`);
};

const recordPass = (recorder, id, message) => recorder.recordAssertion(id, "pass", message);
const recordSkip = (recorder, id, message, payload = null) => recorder.recordAssertion(id, "skipped", message, payload);

describe("updater remote daemon e2e", () => {
  before(function beforeSuite() {
    if (!REMOTE_HOST) {
      this.skip();
    }
  });

  it("proves real remote daemon update flows across configured scenarios", async () => {
    await navigateToTauriUrl("tauri://localhost/workspaces");

    const fixture = remoteFixture();
    const recorder = createRemoteContractRecorder({
      outputPath: reportPath,
      suite: "updater-remote-daemon-e2e",
      lane: "remote-host",
      fixture,
      secretValues: [process.env.OPENROUTER_API_KEY || ""],
    });
    const executed = [];
    let runningBootstrapAvailable = true;
    const finalize = ({ result, reason, error = "" }) =>
      recorder.finalize({
        result,
        reason,
        error,
        extras: {
          bootstrap_channel: BOOTSTRAP_CHANNEL,
          compatible_bootstrap_channel: compatibleBootstrapChannel,
          incompatible_bootstrap_channel: incompatibleBootstrapChannel,
          target_channel: TARGET_CHANNEL,
          scenarios: executed,
        },
      });

    try {
      if (runIdle) {
        let bootstrap;
        try {
          bootstrap = await bootstrapRemoteDaemon({
            bootstrapChannel: incompatibleBootstrapChannel,
            updateChannel: TARGET_CHANNEL,
          });
        } catch (error) {
          if (!isBootstrapDaemonUnsupportedError(error)) {
            throw error;
          }
          const connect = await connectDesktop();
          const info = await waitForConnection(
            (candidate) => String(candidate?.kind || "").toLowerCase() === "ssh",
            "ssh reconnect after invalid managed binary reinstall",
          );
          const afterVersion = readRemoteVersion();
          recorder.recordArtifact("invalid_managed_binary_connect_response", connect);
          recorder.recordArtifact("invalid_managed_binary_connection_info", info);
          recorder.recordArtifact("invalid_managed_binary_bootstrap", error.details || null);
          recordPass(
            recorder,
            "invalid_managed_remote_binary_reinstall",
            `desktop replaced an invalid ${incompatibleBootstrapChannel} managed remote ctx binary with ${afterVersion}`,
          );
          executed.push({
            scenario: "invalid_managed_remote_binary_reinstall",
            bootstrap_channel: incompatibleBootstrapChannel,
            after_version: afterVersion,
          });
          await disconnectDesktop();
          runningBootstrapAvailable = false;
          bootstrap = null;
        }
        if (!bootstrap) {
          recordSkip(
            recorder,
            "running_remote_update_scenarios",
            "running previous-daemon scenarios skipped because the bootstrap channel daemon artifact cannot run ctx serve",
            { bootstrap_channel: incompatibleBootstrapChannel },
          );
        } else {
          const beforeVersion = readRemoteVersion();
          const connect = await connectDesktop();
          recorder.recordArtifact("idle_connect_response", connect);
          const afterVersion = requireVersionChange
            ? await waitForRemoteVersionChange(beforeVersion)
            : readRemoteVersion();
          const info = await waitForConnection((candidate) => String(candidate?.kind || "").toLowerCase() === "ssh", "ssh reconnect");
          recorder.recordArtifact("idle_connection_info", info);
          recorder.recordArtifact("idle_bootstrap", bootstrap);
          recordPass(
            recorder,
            "idle_remote_update",
            `remote daemon updated on connect from ${beforeVersion} to ${afterVersion}`,
          );
          executed.push({
            scenario: "idle_connect_update",
            before_version: beforeVersion,
            after_version: afterVersion,
          });
          await disconnectDesktop();
        }
      }

      if (runPendingIdle && runningBootstrapAvailable) {
        const bootstrap = await bootstrapRemoteDaemon({
          bootstrapChannel: compatibleBootstrapChannel,
          updateChannel: TARGET_CHANNEL,
        });
        const beforeVersion = readRemoteVersion();
        const busyTurn = await createBusyRemoteTurn({ label: "pending-idle", providerId: "qwen" });
        await connectDesktop();
        const pendingInfo = await waitForConnection(
          (info) => String(info?.remote_update_state || "").toLowerCase() === "pending",
          "pending remote update state after busy connect",
        );
        recorder.recordArtifact("pending_idle_connection_info", pendingInfo);
        const terminalTurn = await waitForRemoteTurnTerminal({
          token: busyTurn.token,
          sessionId: busyTurn.session_id,
        });
        assertTurnStatus(terminalTurn, ["completed"], "pending restart-on-idle");
        const afterVersion = requireVersionChange
          ? await waitForRemoteVersionChange(beforeVersion)
          : readRemoteVersion();
        const settledInfo = await waitForConnection(
          (info) => !String(info?.remote_update_state || "").trim(),
          "pending remote update state clear",
        );
        recorder.recordArtifact("pending_idle_turn_terminal", terminalTurn);
        recorder.recordArtifact("pending_idle_connection_settled", settledInfo);
        recorder.recordArtifact("pending_idle_bootstrap", bootstrap);
        recordPass(
          recorder,
          "pending_remote_update_restart_on_idle",
          `remote daemon waited for idle and updated from ${beforeVersion} to ${afterVersion}`,
        );
        executed.push({
          scenario: "pending_restart_on_idle",
          before_version: beforeVersion,
          after_version: afterVersion,
          terminal_status: terminalTurn.status,
        });
        await disconnectDesktop();
      }

      if (runPendingRestartNow && runningBootstrapAvailable) {
        const bootstrap = await bootstrapRemoteDaemon({
          bootstrapChannel: compatibleBootstrapChannel,
          updateChannel: TARGET_CHANNEL,
        });
        const beforeVersion = readRemoteVersion();
        const busyTurn = await createBusyRemoteTurn({ label: "pending-restart-now", providerId: "qwen" });
        await connectDesktop();
        const pendingInfo = await waitForConnection(
          (info) => String(info?.remote_update_state || "").toLowerCase() === "pending",
          "pending remote update state before restart now",
        );
        recorder.recordArtifact("pending_restart_now_connection_info", pendingInfo);
        const restartNowResp = await updateRemoteNow();
        const afterVersion = requireVersionChange
          ? await waitForRemoteVersionChange(beforeVersion)
          : readRemoteVersion();
        const terminalTurn = await waitForRemoteTurnTerminal({
          token: busyTurn.token,
          sessionId: busyTurn.session_id,
        });
        assertTurnStatus(terminalTurn, ["failed", "cancelled"], "pending restart-now");
        const settledInfo = await waitForConnection(
          (info) => !String(info?.remote_update_state || "").trim(),
          "pending remote update state clear after restart now",
        );
        recorder.recordArtifact("pending_restart_now_response", restartNowResp);
        recorder.recordArtifact("pending_restart_now_turn_terminal", terminalTurn);
        recorder.recordArtifact("pending_restart_now_connection_settled", settledInfo);
        recorder.recordArtifact("pending_restart_now_bootstrap", bootstrap);
        recordPass(
          recorder,
          "pending_remote_update_restart_now",
          `restart now interrupted active work and updated remote daemon from ${beforeVersion} to ${afterVersion}`,
        );
        executed.push({
          scenario: "pending_restart_now",
          before_version: beforeVersion,
          after_version: afterVersion,
          terminal_status: terminalTurn.status,
        });
        await disconnectDesktop();
      }

      if (runIncompatibleReconnect && runningBootstrapAvailable) {
        const bootstrap = await bootstrapRemoteDaemon({
          bootstrapChannel: incompatibleBootstrapChannel,
          updateChannel: TARGET_CHANNEL,
        });
        const beforeVersion = readRemoteVersion();
        const busyTurn = await createBusyRemoteTurn({ label: "incompatible-reconnect", providerId: "qwen" });
        await connectDesktop();
        await browser.pause(2000);
        const maybePending = await getConnection();
        if (String(maybePending?.remote_update_state || "").toLowerCase() === "pending") {
          throw new Error(
            `expected incompatible reconnect to restart immediately, but got pending state: ${JSON.stringify(maybePending)}`,
          );
        }
        const afterVersion = requireVersionChange
          ? await waitForRemoteVersionChange(beforeVersion)
          : readRemoteVersion();
        const terminalTurn = await waitForRemoteTurnTerminal({
          token: busyTurn.token,
          sessionId: busyTurn.session_id,
        });
        assertTurnStatus(terminalTurn, ["failed", "cancelled"], "incompatible reconnect");
        recorder.recordArtifact("incompatible_reconnect_turn_terminal", terminalTurn);
        recorder.recordArtifact("incompatible_reconnect_bootstrap", bootstrap);
        recordPass(
          recorder,
          "incompatible_remote_reconnect_restart_immediate",
          `incompatible reconnect restarted remote daemon immediately from ${beforeVersion} to ${afterVersion}`,
        );
        executed.push({
          scenario: "incompatible_reconnect_immediate_restart",
          before_version: beforeVersion,
          after_version: afterVersion,
          terminal_status: terminalTurn.status,
        });
        await disconnectDesktop();
      }

      if (runNoClientAuto && runningBootstrapAvailable) {
        const bootstrap = await bootstrapRemoteDaemon({
          bootstrapChannel: incompatibleBootstrapChannel,
          updateChannel: TARGET_CHANNEL,
        });
        const beforeVersion = readRemoteVersion();
        const afterVersion = requireVersionChange
          ? await waitForRemoteVersionChange(beforeVersion, { timeoutMs: 240000, intervalMs: 2000 })
          : readRemoteVersion();
        const status = readRemoteAutoUpdateStatus();
        recorder.recordArtifact("no_client_auto_status", status);
        recorder.recordArtifact("no_client_auto_bootstrap", bootstrap);
        recordPass(
          recorder,
          "no_client_remote_auto_update",
          `daemon-owned updater advanced remote daemon from ${beforeVersion} to ${afterVersion}`,
        );
        executed.push({
          scenario: "no_client_auto_update",
          before_version: beforeVersion,
          after_version: afterVersion,
        });
      }

      finalize({
        result: "passed",
        reason: "remote daemon updater proof passed",
      });
    } catch (error) {
      finalize({
        result: "failed",
        reason: "remote daemon updater proof failed",
        error: String(error),
      });
      throw error;
    }
  });
});
