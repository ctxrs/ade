const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");
const { ensureCodexOpenRouterWorkspaceReady } = require("./helpers/provider_runtime.cjs");

const REMOTE_HOST = String(process.env.CTX_AUTOMATION_REMOTE_HOST || "").trim();
const REMOTE_USER = String(process.env.CTX_AUTOMATION_REMOTE_USER || "root").trim() || "root";
const REMOTE_PORT = Number.parseInt(String(process.env.CTX_AUTOMATION_REMOTE_PORT || "44099"), 10) || 44099;
const REMOTE_DATA_DIR = String(process.env.CTX_AUTOMATION_REMOTE_DATA_DIR || "").trim();
const REMOTE_CTX_BIN = "$HOME/.ctx/bin/ctx";
const SSH_KEY_PATH = String(
  process.env.CTX_AUTOMATION_REMOTE_SSH_KEY_PATH || process.env.CTX_UPDATER_E2E_SSH_KEY_PATH || "",
).trim();
const SSH_CONFIG_PATH = String(process.env.CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG || "").trim();
const SSH_PORT = Number.parseInt(String(process.env.CTX_AUTOMATION_REMOTE_SSH_PORT || "0"), 10) || 0;
const REMOTE_PASSWORD_ACTUAL = String(
  process.env.CTX_AUTOMATION_REMOTE_PASSWORD_ACTUAL || process.env.CTX_AUTOMATION_REMOTE_PASSWORD || "",
).trim();
const AUTH_TEST_MODE = String(process.env.CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE || "key").trim().toLowerCase();
const EXPECT_CONNECT_FAILURE = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_REMOTE_EXPECT_CONNECT_FAILURE || "0").trim().toLowerCase(),
);
const WRONG_PASSWORD = String(process.env.CTX_AUTOMATION_REMOTE_WRONG_PASSWORD || "definitely-wrong-password").trim();
const FIRST_TURN_REPORT_PATH = String(process.env.CTX_REMOTE_BOOTSTRAP_FIRST_TURN_REPORT || "").trim();
const REQUIRE_FIRST_TURN_SUCCESS = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS || "0").trim().toLowerCase(),
);
const SKIP_MANAGED_BINARY_RESET = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_REMOTE_SKIP_MANAGED_BINARY_RESET || "0").trim().toLowerCase(),
);

const tauriInvoke = async (command, args) => {
  try {
    const result = await browser.executeAsync(({ cmd, payload }, done) => {
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
    return result && typeof result === "object" ? result : { value: result };
  } catch (err) {
    return { error: String(err) };
  }
};

const connectSshWithPolling = async (req, timeoutMs = 240000) => {
  const begin = await tauriInvoke("desktop_connect_ssh_begin", { req });
  if (begin.error) {
    const detail = String(begin.error || "").toLowerCase();
    // Backward compatibility with older desktop builds that only expose desktop_connect_ssh.
    if (detail.includes("desktop_connect_ssh_begin") || detail.includes("unknown command")) {
      return tauriInvoke("desktop_connect_ssh", { req });
    }
    return begin;
  }

  const jobId = String(begin.value || "").trim();
  if (!jobId) {
    return { error: "desktop_connect_ssh_begin returned empty job id" };
  }

  const startedAt = Date.now();
  let lastError = "";
  while (Date.now() - startedAt < timeoutMs) {
    const poll = await tauriInvoke("desktop_connect_ssh_poll", {
      req: { job_id: jobId, consume: false },
    });
    if (poll.error) {
      lastError = String(poll.error || "");
      await browser.pause(500);
      continue;
    }
    const snapshot = poll.value || {};
    const status = String(snapshot.status || "").trim().toLowerCase();
    if (status === "succeeded") {
      await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
      return { value: snapshot.info || null };
    }
    if (status === "failed") {
      await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
      const detail = String(snapshot.error || "desktop_connect_ssh failed");
      return { error: detail };
    }
    await browser.pause(500);
  }

  await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
  return {
    error: `desktop_connect_ssh timed out waiting for async completion${lastError ? `: ${lastError}` : ""}`,
  };
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
  if (SSH_CONFIG_PATH) {
    args.unshift(SSH_CONFIG_PATH);
    args.unshift("-F");
  } else {
    args.unshift("/dev/null");
    args.unshift("-F");
  }
  if (SSH_PORT > 0) {
    args.push("-p", String(SSH_PORT));
  }
  if (useKey && SSH_KEY_PATH) {
    args.unshift("IdentitiesOnly=yes");
    args.unshift("-o");
    args.unshift(SSH_KEY_PATH);
    args.unshift("-i");
  }
  return args;
};

const remoteSsh = (command, { auth = "key", passwordOverride = "" } = {}) => {
  const target = `${REMOTE_USER}@${REMOTE_HOST}`;
  const usePassword = auth === "password";
  const args = sshBaseArgs({ useKey: !usePassword });
  if (usePassword) {
    const pass = String(passwordOverride || REMOTE_PASSWORD_ACTUAL || "").trim();
    if (!pass) {
      throw new Error("password auth requested but fixture password is not set");
    }
    return String(
      execFileSync(
        "sshpass",
        [
          "-e",
          "ssh",
          ...args,
          "-o",
          "BatchMode=no",
          "-o",
          "PreferredAuthentications=password,keyboard-interactive",
          "-o",
          "NumberOfPasswordPrompts=1",
          target,
          command,
        ],
        {
          encoding: "utf8",
          stdio: ["ignore", "pipe", "pipe"],
          env: { ...process.env, SSHPASS: pass },
        },
      ) || "",
    ).trim();
  }
  return String(execFileSync("ssh", [...args, "-o", "BatchMode=yes", target, command], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }) || "").trim();
};

const diagnosticsAuthMode = () => {
  if (AUTH_TEST_MODE === "password_once" || AUTH_TEST_MODE === "wrong_password") return "password";
  return "key";
};

const managedState = () => {
  const cmd = `if [ -x ${REMOTE_CTX_BIN} ]; then echo present; else echo missing; fi`;
  return remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, { auth: diagnosticsAuthMode() });
};

const removeManagedBinary = () => {
  const cmd = `rm -f ${REMOTE_CTX_BIN}`;
  remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, { auth: diagnosticsAuthMode() });
};

const stopRemoteDaemon = () => {
  const cmd = "if command -v pkill >/dev/null 2>&1; then pkill -x ctx >/dev/null 2>&1 || true; fi; true";
  remoteSsh(`sh -lc ${JSON.stringify(cmd)}`, { auth: diagnosticsAuthMode() });
};

const ensureRemoteWorkspaceRepo = () => {
  const remoteRoot = `/tmp/ctx-remote-bootstrap-contract-${Date.now()}-${Math.trunc(Math.random() * 100000)}`;
  const setupCmd = [
    `set -euo pipefail`,
    `mkdir -p ${JSON.stringify(remoteRoot)}`,
    `cd ${JSON.stringify(remoteRoot)}`,
    `if [ ! -d .git ]; then git init >/dev/null 2>&1; fi`,
    `if [ ! -f README.md ]; then printf '%s\n' '# remote bootstrap contract' > README.md; fi`,
    `if ! git rev-parse --verify HEAD >/dev/null 2>&1; then git add README.md && git -c user.name='ctx fixture' -c user.email='ctx-fixture@example.invalid' commit -m 'bootstrap fixture repo' >/dev/null 2>&1; fi`,
  ].join("; ");
  remoteSsh(`bash -lc ${JSON.stringify(setupCmd)}`, { auth: diagnosticsAuthMode() });
  return remoteRoot;
};

const connectReq = () => {
  const req = {
    host: REMOTE_HOST,
    user: REMOTE_USER,
    remote_port: REMOTE_PORT,
    start_remote: true,
    remote_data_dir: REMOTE_DATA_DIR || undefined,
  };
  if (AUTH_TEST_MODE === "password_once") {
    req.password_once = REMOTE_PASSWORD_ACTUAL;
  } else if (AUTH_TEST_MODE === "wrong_password") {
    req.password_once = WRONG_PASSWORD;
  } else {
    const authMode = String(process.env.CTX_AUTOMATION_REMOTE_AUTH_MODE || "").trim().toLowerCase();
    if (authMode === "password" && !EXPECT_CONNECT_FAILURE && REMOTE_PASSWORD_ACTUAL) {
      req.password_once = REMOTE_PASSWORD_ACTUAL;
    }
  }
  return req;
};

const assertRemoteHealth = async (stage) => {
  const health = await daemonJson("GET", "/api/health");
  if (health.status !== 200) {
    throw new Error(`${stage}: expected /api/health 200, got ${health.status} payload=${JSON.stringify(health.payload || null)}`);
  }
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
  return { workspaceId, remoteRoot };
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

const runFirstTurnOutcome = async (
  workspaceId,
  {
    providerId = "codex",
    modelId = "default",
  } = {},
  timeoutMs = 180000,
) => {
  const taskResp = await daemonJson("POST", `/api/workspaces/${workspaceId}/tasks`, {
    title: `remote-bootstrap-first-turn-${Date.now()}`,
    description: "Remote bootstrap first-turn contract",
    create_default_session: false,
  });
  if (taskResp.status !== 200) {
    return { status: "failed", stage: "task_create", detail: JSON.stringify(taskResp.payload || null) };
  }
  const taskId = String(taskResp.payload?.id || "").trim();
  if (!taskId) {
    return { status: "failed", stage: "task_create", detail: "missing task id" };
  }

  const sessionResp = await daemonJson("POST", `/api/tasks/${taskId}/sessions`, {
    provider_id: providerId,
    model_id: modelId,
    env_target: "worktree",
  });
  if (sessionResp.status !== 200) {
    return { status: "failed", stage: "session_create", detail: JSON.stringify(sessionResp.payload || null) };
  }
  const sessionId = String(sessionResp.payload?.id || "").trim();
  if (!sessionId) {
    return { status: "failed", stage: "session_create", detail: "missing session id" };
  }

  const postResp = await daemonJson("POST", `/api/sessions/${sessionId}/messages`, {
    content: "hello",
    delivery: "immediate",
    attachments: [],
  });
  if (postResp.status !== 200) {
    return { status: "failed", stage: "message_post", detail: JSON.stringify(postResp.payload || null), session_id: sessionId };
  }

  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const history = await daemonJson("GET", `/api/sessions/${sessionId}/history?limit=200`);
    if (history.status === 200 && history.payload) {
      const messages = Array.isArray(history.payload.messages) ? history.payload.messages : [];
      const assistantMessage = messages
        .filter((m) => String(m?.role || "").toLowerCase() === "assistant")
        .map((m) => String(m?.content || "").trim())
        .find((content) => content.length > 0) || "";
      if (assistantMessage) {
        return {
          status: "success",
          session_id: sessionId,
          assistant_preview: assistantMessage.slice(0, 200),
        };
      }

      const turns = Array.isArray(history.payload.turns) ? history.payload.turns : [];
      const latestTurn = turns.length ? turns[turns.length - 1] : null;
      const latestStatus = String(latestTurn?.status || "").trim().toLowerCase();
      if (latestStatus === "failed" || latestStatus === "cancelled") {
        return {
          status: "failed",
          stage: "turn",
          turn_status: latestStatus,
          session_id: sessionId,
          detail: String(latestTurn?.error || latestTurn?.error_message || "turn failed"),
        };
      }
    }
    await browser.pause(500);
  }
  return { status: "timed_out", stage: "turn", session_id: null, detail: "no assistant output before timeout" };
};

describe("remote bootstrap install e2e", () => {
  before(function () {
    if (!REMOTE_HOST) {
      this.skip();
    }
  });

  it("connects over SSH and validates managed remote daemon bootstrap contracts", async () => {
    await browser.url("tauri://localhost/workspaces");
    await waitForTauri();

    if (!SKIP_MANAGED_BINARY_RESET) {
      removeManagedBinary();
      const beforeState = managedState();
      if (beforeState !== "missing") {
        throw new Error(`expected managed binary to be missing before connect, got '${beforeState}'`);
      }
    }

    const req = connectReq();
    const connectResp = await connectSshWithPolling(req);

    if (EXPECT_CONNECT_FAILURE || AUTH_TEST_MODE === "wrong_password") {
      if (!connectResp.error) {
        throw new Error(`expected connect failure for wrong-password mode, got success: ${JSON.stringify(connectResp.value || null)}`);
      }
      const detail = String(connectResp.error || "");
      if (!/password-once ssh bootstrap failed|permission denied|failed to reach remote daemon/i.test(detail)) {
        throw new Error(`expected password auth failure detail, got: ${detail}`);
      }
      return;
    }

    if (connectResp.error) {
      throw new Error(`desktop_connect_ssh failed: ${connectResp.error}`);
    }

    const connectionResp = await tauriInvoke("desktop_get_connection", {});
    if (connectionResp.error) {
      throw new Error(`desktop_get_connection failed: ${connectionResp.error}`);
    }
    const kind = String(connectionResp.value?.kind || "").toLowerCase();
    if (kind !== "ssh") {
      throw new Error(`expected ssh connection after bootstrap, got ${JSON.stringify(connectionResp.value)}`);
    }

    const afterState = managedState();
    if (afterState !== "present") {
      throw new Error(`expected managed binary at ${REMOTE_CTX_BIN}, got '${afterState}'`);
    }

    const helpCmd = `if [ -x ${REMOTE_CTX_BIN} ]; then ${REMOTE_CTX_BIN} --help; else echo missing; fi`;
    const helpOutput = remoteSsh(`sh -lc ${JSON.stringify(helpCmd)}`, { auth: diagnosticsAuthMode() });
    if (!helpOutput || helpOutput === "missing" || !helpOutput.includes("Usage: ctx")) {
      throw new Error(`expected installed managed binary to execute and print usage, got '${helpOutput}'`);
    }

    await assertRemoteHealth("post-bootstrap");
    const launch = await assertWorkspaceLaunch();

    let firstTurnProvider = null;
    let firstTurn = null;
    try {
      firstTurnProvider = await ensureCodexOpenRouterWorkspaceReady(launch.workspaceId, {
        installTarget: "host",
        endpointName: `remote-bootstrap-openrouter-${Date.now()}`,
      });
    } catch (error) {
      if (REQUIRE_FIRST_TURN_SUCCESS) {
        throw new Error(`failed to configure remote first-turn provider auth: ${String(error)}`);
      }
      firstTurn = {
        status: "failed",
        stage: "provider_setup",
        detail: String(error),
      };
    }

    if (!firstTurn) {
      firstTurn = await runFirstTurnOutcome(
        launch.workspaceId,
        {
          providerId: firstTurnProvider?.providerId || "codex",
          modelId: firstTurnProvider?.modelId || "default",
        },
      );
    }
    writeFirstTurnReport({
      workspace_id: launch.workspaceId,
      workspace_root: launch.remoteRoot,
      auth_mode: AUTH_TEST_MODE,
      provider_config: firstTurnProvider,
      first_turn: firstTurn,
    });
    if (REQUIRE_FIRST_TURN_SUCCESS && firstTurn.status !== "success") {
      throw new Error(`expected first turn success, got ${JSON.stringify(firstTurn)}`);
    }

    // Reconnect path: disconnect and reconnect without password_once. This must work
    // for both key mode and password-once mode after key bootstrap completes.
    const disconnectResp = await tauriInvoke("desktop_disconnect", {});
    if (disconnectResp.error) {
      throw new Error(`desktop_disconnect failed: ${disconnectResp.error}`);
    }
    const reconnectResp = await connectSshWithPolling({
      host: REMOTE_HOST,
      user: REMOTE_USER,
      remote_port: REMOTE_PORT,
      start_remote: true,
      remote_data_dir: REMOTE_DATA_DIR || undefined,
    });
    if (reconnectResp.error) {
      throw new Error(`desktop reconnect failed: ${reconnectResp.error}`);
    }
    await assertRemoteHealth("post-reconnect");

    // Restart path: kill remote daemon process and verify desktop reconnect can restart it.
    stopRemoteDaemon();
    const disconnectResp2 = await tauriInvoke("desktop_disconnect", {});
    if (disconnectResp2.error) {
      throw new Error(`desktop_disconnect before restart check failed: ${disconnectResp2.error}`);
    }
    const restartResp = await connectSshWithPolling({
      host: REMOTE_HOST,
      user: REMOTE_USER,
      remote_port: REMOTE_PORT,
      start_remote: true,
      remote_data_dir: REMOTE_DATA_DIR || undefined,
    });
    if (restartResp.error) {
      throw new Error(`desktop reconnect after remote daemon stop failed: ${restartResp.error}`);
    }
    await assertRemoteHealth("post-remote-daemon-restart");
  });
});
