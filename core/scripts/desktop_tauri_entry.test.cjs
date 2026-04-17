#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createInvocation,
  normalizeTauriCliEnv,
  resolvePrepMode,
  shouldSkipPrep,
} = require("./desktop_tauri_entry.cjs");

test("desktop_tauri_entry uses release prep for normal builds", () => {
  const invocation = createInvocation(["node", "desktop_tauri_entry", "build", "--bundles", "app"]);
  assert.equal(invocation.prepMode, "release-build");
  assert.deepEqual(invocation.prepArgs, ["scripts/desktop_prepare.cjs", "--mode", "release-build"]);
  assert.match(invocation.tauriCommand, /core\/apps\/desktop\/node_modules\/\.bin\/tauri$/);
  assert.deepEqual(invocation.tauriExecArgs, ["build", "--bundles", "app"]);
});

test("desktop_tauri_entry strips only the outer pnpm separator", () => {
  const invocation = createInvocation([
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
  ]);
  assert.equal(invocation.prepMode, "debug-build");
  assert.deepEqual(invocation.tauriExecArgs, ["build", "--debug", "--bundles", "app", "--", "--features", "automation"]);
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

test("desktop_tauri_entry normalizes CI=1 for the tauri CLI", () => {
  const normalized = normalizeTauriCliEnv({
    CI: "1",
    OTHER: "value",
  });
  assert.equal(normalized.CI, "true");
  assert.equal(normalized.OTHER, "value");
});
