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
