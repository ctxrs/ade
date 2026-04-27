#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const {
  createInvocation,
  normalizeTauriCliEnv,
  resolvePrepMode,
  resolveTauriBudgetKey,
  shouldSkipPrep,
} = require("./desktop_tauri_entry.cjs");
const { HOST_HEAVY_BUDGET_KEY } = require("./lib/host_job_budget.cjs");
const { readDesktopVersion } = require("./desktop_version.cjs");

const currentDesktopVersion = readDesktopVersion(path.resolve(__dirname, ".."));

test("desktop_tauri_entry uses release prep for normal builds", () => {
  const invocation = createInvocation(["node", "desktop_tauri_entry", "build", "--bundles", "app"]);
  assert.equal(invocation.prepMode, "release-build");
  assert.deepEqual(invocation.prepArgs, ["scripts/desktop_prepare.cjs", "--mode", "release-build"]);
  assert.equal(invocation.tauriBudgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.match(invocation.tauriCommand, /core\/apps\/desktop\/node_modules\/\.bin\/tauri$/);
  assert.equal(invocation.tauriExecArgs[0], "build");
  assert.equal(invocation.tauriExecArgs[1], "--config");
  assert.match(invocation.tauriExecArgs[2], /tauri\.identity\.json$/);
  assert.deepEqual(invocation.tauriExecArgs.slice(3), ["--bundles", "app"]);
  assert.equal(invocation.tauriEnv.CTX_RELEASE_EFFECTIVE_VERSION, currentDesktopVersion);
});

test("desktop_tauri_entry strips only the outer pnpm separator", () => {
  const invocation = createInvocation(
    [
      "node",
      "desktop_tauri_entry",
      "build",
      "--",
      "--debug",
      "--bundles",
      "app",
      "--",
      "--features",
      "automation",
    ],
    {
      CTX_BUILD_ID: "localpkg123",
    },
  );
  assert.equal(invocation.prepMode, "debug-build");
  assert.equal(invocation.tauriExecArgs[0], "build");
  assert.equal(invocation.tauriExecArgs[1], "--config");
  assert.match(invocation.tauriExecArgs[2], /tauri\.identity\.json$/);
  assert.deepEqual(invocation.tauriExecArgs.slice(3), ["--debug", "--bundles", "app", "--", "--features", "automation"]);
  assert.equal(invocation.tauriEnv.CTX_BUILD_ID, "localpkg123");
  assert.equal(invocation.tauriEnv.CTX_COMPATIBILITY_TOKEN, "artifact-localpkg123");
});

test("desktop_tauri_entry uses debug prep for debug builds", () => {
  assert.equal(
    resolvePrepMode({ command: "build", tauriArgs: ["--debug", "--bundles", "app"] }),
    "debug-build",
  );
});

test("desktop_tauri_entry uses dev prep for tauri dev", () => {
  const invocation = createInvocation(["node", "desktop_tauri_entry", "dev", "--no-watch"]);
  assert.equal(invocation.prepMode, "dev");
  assert.equal(invocation.tauriBudgetKey, "");
  assert.deepEqual(invocation.tauriExecArgs, ["dev", "--no-watch"]);
});

test("desktop_tauri_entry can skip prep when the caller already prepared resources", () => {
  const invocation = createInvocation(
    ["node", "desktop_tauri_entry", "build", "--bundles", "appimage"],
    { CTX_DESKTOP_SKIP_PREP: "1" },
  );
  assert.equal(shouldSkipPrep({ CTX_DESKTOP_SKIP_PREP: "1" }), true);
  assert.equal(invocation.skipPrep, true);
  assert.equal(invocation.prepCommand, "");
  assert.deepEqual(invocation.prepArgs, []);
});

test("desktop_tauri_entry derives release identity for canary builds", () => {
  const invocation = createInvocation(
    ["node", "desktop_tauri_entry", "build", "--bundles", "app"],
    {
      RELEASE_CHANNEL: "canary",
      RELEASE_SOURCE_COMMIT: "deadbeefcafebabefeedface1234567890abcdef",
    },
  );
  assert.equal(
    invocation.tauriEnv.CTX_RELEASE_EFFECTIVE_VERSION,
    `${currentDesktopVersion}-canary.deadbeefcafe`,
  );
  assert.equal(invocation.tauriEnv.CTX_BUILD_ID, "deadbeefcafe");
  assert.equal(
    invocation.tauriEnv.CTX_COMPATIBILITY_TOKEN,
    "artifact-deadbeefcafebabefeedface1234567890abcdef",
  );
});

test("desktop_tauri_entry normalizes CI=1 for the tauri CLI", () => {
  const normalized = normalizeTauriCliEnv({
    CI: "1",
    OTHER: "value",
  });
  assert.equal(normalized.CI, "true");
  assert.equal(normalized.OTHER, "value");
});

test("desktop_tauri_entry only budgets build commands", () => {
  assert.equal(resolveTauriBudgetKey("build"), HOST_HEAVY_BUDGET_KEY);
  assert.equal(resolveTauriBudgetKey("dev"), "");
});
