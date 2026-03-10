const test = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("crypto");

const HELPER_PATH = require.resolve("./specs/helpers/container_lifecycle.cjs");
const TAURI_HELPER_PATH = require.resolve("./specs/helpers/tauri.cjs");
const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");
const WORKSPACE_HELPER_PATH = require.resolve("./specs/helpers/workspace_wizard_flow.cjs");

const loadHelper = () => {
  delete require.cache[HELPER_PATH];
  delete require.cache[TAURI_HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  delete require.cache[WORKSPACE_HELPER_PATH];

  require.cache[TAURI_HELPER_PATH] = {
    id: TAURI_HELPER_PATH,
    filename: TAURI_HELPER_PATH,
    loaded: true,
    exports: {
      getConnectionInfo: () => ({ port: 0 }),
    },
  };
  require.cache[DAEMON_HELPER_PATH] = {
    id: DAEMON_HELPER_PATH,
    filename: DAEMON_HELPER_PATH,
    loaded: true,
    exports: {
      daemonJson: async () => {
        throw new Error("daemonJson not stubbed for this test");
      },
      safeDaemonJson: async () => null,
    },
  };
  require.cache[WORKSPACE_HELPER_PATH] = {
    id: WORKSPACE_HELPER_PATH,
    filename: WORKSPACE_HELPER_PATH,
    loaded: true,
    exports: {
      initGitRepo: async () => {
        throw new Error("initGitRepo not stubbed for this test");
      },
      collectWorkspaceRouteDiagnostics: async () => ({}),
      collectCodexSmokeDiagnostics: async () => ({}),
    },
  };

  return require(HELPER_PATH);
};

test.afterEach(() => {
  delete require.cache[HELPER_PATH];
  delete require.cache[TAURI_HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  delete require.cache[WORKSPACE_HELPER_PATH];
});

test("ctxPodmanMachineName matches the runtime machine hash length", () => {
  const { ctxPodmanMachineName } = loadHelper();
  const dataRoot = "/tmp/ctx-desktop-e2e-sample-daemon";
  const expectedHash = crypto
    .createHash("sha256")
    .update(dataRoot)
    .digest("hex")
    .slice(0, 12);

  assert.equal(ctxPodmanMachineName(dataRoot), `ctx-${expectedHash}`);
  assert.match(ctxPodmanMachineName(dataRoot), /^ctx-[a-f0-9]{12}$/);
});
