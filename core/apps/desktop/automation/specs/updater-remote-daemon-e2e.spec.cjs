const { execFileSync } = require("child_process");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { resolveBoolishFlag } = require("../../../../scripts/lib/boolish.cjs");

const REMOTE_HOST = String(
  process.env.CTX_AUTOMATION_REMOTE_HOST || process.env.CTX_UPDATER_E2E_REMOTE_HOST || "",
).trim();
const REMOTE_USER = String(process.env.CTX_AUTOMATION_REMOTE_USER || "root").trim() || "root";
const REMOTE_PORT = Number.parseInt(String(process.env.CTX_AUTOMATION_REMOTE_PORT || "44099"), 10) || 44099;
const REMOTE_DATA_DIR = String(process.env.CTX_AUTOMATION_REMOTE_DATA_DIR || "").trim();
const REMOTE_CTX_BIN = "$HOME/.ctx/bin/ctx";
const REMOTE_CHANNEL = String(
  process.env.CTX_UPDATER_E2E_REMOTE_CHANNEL || process.env.RELEASE_CHANNEL || "stable",
).trim() || "stable";
const SSH_KEY_PATH = String(
  process.env.CTX_UPDATER_E2E_SSH_KEY_PATH || process.env.CTX_AUTOMATION_REMOTE_SSH_KEY_PATH || "",
).trim();
const EXPECT_VERSION_CHANGE = resolveBoolishFlag(
  process.env.CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE,
  false,
  "CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE",
);
const SSH_CONFIG_PATH = String(process.env.CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG || "").trim();
const SSH_PORT = Number.parseInt(String(process.env.CTX_AUTOMATION_REMOTE_SSH_PORT || "0"), 10) || 0;

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

const remoteSsh = (command) => {
  const target = `${REMOTE_USER}@${REMOTE_HOST}`;
  const args = [
    "-o",
    "BatchMode=yes",
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
  if (SSH_KEY_PATH) {
    args.unshift(SSH_KEY_PATH);
    args.unshift("-i");
  }
  args.push(target, command);
  return String(execFileSync("ssh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }) || "").trim();
};

const remoteVersion = () => {
  const cmd = `if [ -x ${REMOTE_CTX_BIN} ]; then ${REMOTE_CTX_BIN} --version; else echo missing; fi`;
  return remoteSsh(`sh -lc ${JSON.stringify(cmd)}`);
};

describe("updater remote daemon e2e", () => {
  before(function () {
    if (!REMOTE_HOST) {
      this.skip();
    }
  });

  it("connects over SSH, updates remote daemon, and remains connected", async () => {
    await browser.url("tauri://localhost/workspaces");
    await waitForTauri();

    const beforeVersion = remoteVersion();

    const connectResp = await tauriInvoke("desktop_connect_ssh", {
      req: {
        host: REMOTE_HOST,
        user: REMOTE_USER,
        remote_port: REMOTE_PORT,
        start_remote: true,
        remote_data_dir: REMOTE_DATA_DIR || undefined,
      },
    });
    if (connectResp.error) {
      throw new Error(`desktop_connect_ssh failed: ${connectResp.error}`);
    }

    const updateResp = await tauriInvoke("desktop_update_remote_daemon", {
      req: {
        confirm: true,
        channel: REMOTE_CHANNEL,
      },
    });
    if (updateResp.error) {
      throw new Error(`desktop_update_remote_daemon failed: ${updateResp.error}`);
    }

    const connectionResp = await tauriInvoke("desktop_get_connection", {});
    if (connectionResp.error) {
      throw new Error(`desktop_get_connection failed: ${connectionResp.error}`);
    }
    const kind = String(connectionResp.value?.kind || "").toLowerCase();
    if (kind !== "ssh") {
      throw new Error(`expected ssh connection after remote update, got ${JSON.stringify(connectionResp.value)}`);
    }

    const afterVersion = remoteVersion();
    if (EXPECT_VERSION_CHANGE && beforeVersion === afterVersion) {
      throw new Error(`expected remote version change but remained '${beforeVersion}'`);
    }

    if (!/updated/i.test(String(updateResp.value?.message || ""))) {
      throw new Error(`unexpected update response: ${JSON.stringify(updateResp.value)}`);
    }
  });
});
