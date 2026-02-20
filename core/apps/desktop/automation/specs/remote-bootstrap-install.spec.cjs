const { execFileSync } = require("child_process");

const { waitForTauri } = require("./helpers/tauri.cjs");

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

const tauriInvoke = async (command, args) => {
  try {
    const result = await browser.execute(async ({ cmd, payload }) => {
      const tauriInvoke = window.__TAURI__?.core?.invoke;
      const internalsInvoke = window.__TAURI_INTERNALS__?.invoke;
      const invoke = internalsInvoke || tauriInvoke;
      if (!invoke) {
        return { error: "Tauri invoke API not available" };
      }
      try {
        const value = await invoke(cmd, payload);
        return { value };
      } catch (err) {
        return { error: String(err) };
      }
    }, { cmd: command, payload: args });
    return result && typeof result === "object" ? result : { value: result };
  } catch (err) {
    return { error: String(err) };
  }
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
    args.unshift("IdentitiesOnly=yes");
    args.unshift("-o");
    args.unshift(SSH_KEY_PATH);
    args.unshift("-i");
  }
  args.push(target, command);
  return String(execFileSync("ssh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }) || "").trim();
};

const managedState = () => {
  const cmd = `if [ -x ${REMOTE_CTX_BIN} ]; then echo present; else echo missing; fi`;
  return remoteSsh(`sh -lc ${JSON.stringify(cmd)}`);
};

const removeManagedBinary = () => {
  const cmd = `rm -f ${REMOTE_CTX_BIN}`;
  remoteSsh(`sh -lc ${JSON.stringify(cmd)}`);
};

describe("remote bootstrap install e2e", () => {
  before(function () {
    if (!REMOTE_HOST) {
      this.skip();
    }
  });

  it("connects over SSH and installs managed remote daemon binary", async () => {
    await browser.url("tauri://localhost/workspaces");
    await waitForTauri();

    removeManagedBinary();
    const beforeState = managedState();
    if (beforeState !== "missing") {
      throw new Error(`expected managed binary to be missing before connect, got '${beforeState}'`);
    }

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
      throw new Error(`expected managed binary install at ${REMOTE_CTX_BIN}, got '${afterState}'`);
    }

    const helpCmd = `if [ -x ${REMOTE_CTX_BIN} ]; then ${REMOTE_CTX_BIN} --help; else echo missing; fi`;
    const helpOutput = remoteSsh(`sh -lc ${JSON.stringify(helpCmd)}`);
    if (!helpOutput || helpOutput === "missing" || !helpOutput.includes("Usage: ctx")) {
      throw new Error(`expected installed managed binary to execute and print usage, got '${helpOutput}'`);
    }
  });
});
