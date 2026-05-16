const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const modulePath = require.resolve("./wdio.conf.cjs");

const withEnv = async (overrides, fn) => {
  const previous = new Map();
  for (const key of Object.keys(overrides)) {
    previous.set(key, process.env[key]);
    const value = overrides[key];
    if (value === null) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }

  delete require.cache[modulePath];
  try {
    return await fn(require("./wdio.conf.cjs"));
  } finally {
    delete require.cache[modulePath];
    for (const [key, value] of previous.entries()) {
      if (typeof value === "undefined") {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  }
};

test("wdio mocha timeout defaults to the long case budget", async () => {
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-timeout-test-"));
  try {
    await withEnv(
      {
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
        CTX_AUTOMATION_MOCHA_TIMEOUT_MS: null,
        CTX_AUTOMATION_CASE_TIMEOUT_MS: null,
      },
      (mod) => {
        assert.equal(mod.__desktopAutomationConfigTestHooks.resolveMochaTimeoutMs(), 1200000);
        assert.equal(mod.config.mochaOpts.timeout, 1200000);
      },
    );
  } finally {
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio mocha timeout follows explicit automation override before case timeout", async () => {
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-timeout-test-"));
  try {
    await withEnv(
      {
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
        CTX_AUTOMATION_MOCHA_TIMEOUT_MS: "765432",
        CTX_AUTOMATION_CASE_TIMEOUT_MS: "654321",
      },
      (mod) => {
        assert.equal(mod.__desktopAutomationConfigTestHooks.resolveMochaTimeoutMs(), 765432);
        assert.equal(mod.config.mochaOpts.timeout, 765432);
      },
    );
  } finally {
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio mocha timeout inherits case timeout when mocha override is unset", async () => {
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-timeout-test-"));
  try {
    await withEnv(
      {
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
        CTX_AUTOMATION_MOCHA_TIMEOUT_MS: null,
        CTX_AUTOMATION_CASE_TIMEOUT_MS: "654321",
      },
      (mod) => {
        assert.equal(mod.__desktopAutomationConfigTestHooks.resolveMochaTimeoutMs(), 654321);
        assert.equal(mod.config.mochaOpts.timeout, 654321);
      },
    );
  } finally {
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio connection retry budget is explicitly configurable for slow packaged app startup", async () => {
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-timeout-test-"));
  try {
    await withEnv(
      {
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
        CTX_AUTOMATION_CONNECTION_RETRY_COUNT: "18",
        CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS: "240000",
      },
      (mod) => {
        assert.equal(mod.__desktopAutomationConfigTestHooks.resolveConnectionRetryCount(), 18);
        assert.equal(mod.__desktopAutomationConfigTestHooks.resolveConnectionRetryTimeoutMs(), 240000);
        assert.equal(mod.config.connectionRetryCount, 18);
        assert.equal(mod.config.connectionRetryTimeout, 240000);
      },
    );
  } finally {
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio app builds route through desktop_tauri_entry so resources are prepared", async () => {
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-timeout-test-"));
  try {
    await withEnv(
      {
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
        CTX_AUTOMATION_TAURI_BUNDLES: "app",
        CTX_DESKTOP_APP_PATH: process.platform === "darwin" ? "/tmp/ctx.app" : "/tmp/ctx",
        RUSTC_WRAPPER: "/usr/bin/sccache",
        SCCACHE_DIR: "/tmp/deep/sccache-dir",
        SCCACHE_PATH: "/usr/bin/sccache",
        SCCACHE_SERVER_UDS: "/tmp/deep/sccache/socket/path/that/must/not/leak.sock",
      },
      (mod) => {
        const invocation = mod.__desktopAutomationConfigTestHooks.createAppBuildInvocation();
        assert.equal(invocation.command, process.execPath);
        assert.match(invocation.args[0], /scripts\/desktop_tauri_entry\.cjs$/);
        assert.deepEqual(invocation.args.slice(-3), ["--", "--features", "automation"]);
        assert.equal(invocation.args[1], "build");
        assert.equal(invocation.args[2], "--debug");
        assert.equal(invocation.env.CARGO_INCREMENTAL, "0");
        assert.equal(invocation.env.CTX_DESKTOP_SYNC_BUNDLES, "0");
        assert.equal(invocation.env.RUSTC_WRAPPER, undefined);
        assert.equal(invocation.env.SCCACHE_DIR, undefined);
        assert.equal(invocation.env.SCCACHE_PATH, undefined);
        assert.equal(invocation.env.SCCACHE_SERVER_UDS, undefined);
        if (process.platform === "darwin") {
          assert.deepEqual(invocation.args.slice(3, 5), ["--bundles", "app"]);
        } else {
          assert.equal(invocation.args[3], "--no-bundle");
        }
      },
    );
  } finally {
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});
