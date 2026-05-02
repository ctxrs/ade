const test = require("node:test");
const assert = require("node:assert/strict");

const HELPER_PATH = require.resolve("./specs/helpers/workspace_wizard_flow.cjs");
const TAURI_HELPER_PATH = require.resolve("./specs/helpers/tauri.cjs");
const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");

const loadHelper = () => {
  delete require.cache[HELPER_PATH];
  delete require.cache[TAURI_HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];

  require.cache[TAURI_HELPER_PATH] = {
    id: TAURI_HELPER_PATH,
    filename: TAURI_HELPER_PATH,
    loaded: true,
    exports: {
      waitForTauri: async () => {},
      selectorForTestId: (value) => `[data-testid="${value}"]`,
      waitForTestId: async () => {},
      clickTestId: async () => {},
      setInputTestId: async () => {},
      getConnectionInfo: () => ({ port: 0 }),
    },
  };
  require.cache[DAEMON_HELPER_PATH] = {
    id: DAEMON_HELPER_PATH,
    filename: DAEMON_HELPER_PATH,
    loaded: true,
    exports: {
      daemonJson: async () => ({ status: 200, payload: {} }),
      daemonJsonOnce: async () => ({ status: 200, payload: {} }),
      safeDaemonJson: async () => ({ status: 200, payload: {} }),
    },
  };

  return require(HELPER_PATH);
};

test.afterEach(() => {
  delete require.cache[HELPER_PATH];
  delete require.cache[TAURI_HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  delete global.browser;
});

test("waitForAnyStep tolerates a transient missing wizard root during remote step settle", async () => {
  const { waitForAnyStep } = loadHelper();
  const steps = [null, null, "source"];

  global.browser = {
    execute: async () => steps.shift() ?? "source",
    waitUntil: async (predicate, { timeoutMsg }) => {
      for (let i = 0; i < 10; i += 1) {
        if (await predicate()) return true;
      }
      throw new Error(timeoutMsg);
    },
  };

  const step = await waitForAnyStep(["container", "source", "harness-downloads"], 1000);
  assert.equal(step, "source");
});

test("clickOption succeeds when the wizard advances before the target option is observed", async () => {
  const { clickOption } = loadHelper();

  global.browser = {
    execute: async () => ({
      step: "container",
      done: true,
      reason: "step_changed",
      optionPresent: false,
      selectedOptionId: "",
      hasSourcePath: false,
      hasRepoUrl: false,
      hasWorkspaceName: false,
      optionTestIds: [],
    }),
    pause: async () => {},
  };

  await assert.doesNotReject(() => clickOption("location", "local"));
});
