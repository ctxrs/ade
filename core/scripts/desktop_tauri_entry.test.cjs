#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createInvocation,
  resolvePrepMode,
} = require("./desktop_tauri_entry.cjs");

test("desktop_tauri_entry uses release prep for normal builds", () => {
  const invocation = createInvocation(["node", "desktop_tauri_entry", "build", "--bundles", "app"]);
  assert.equal(invocation.prepMode, "release-build");
  assert.deepEqual(invocation.prepArgs, ["scripts/desktop_prepare.cjs", "--mode", "release-build"]);
  assert.deepEqual(invocation.tauriExecArgs, [
    "-C",
    "apps/desktop",
    "exec",
    "tauri",
    "build",
    "--bundles",
    "app",
  ]);
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
  assert.deepEqual(invocation.tauriExecArgs, [
    "-C",
    "apps/desktop",
    "exec",
    "tauri",
    "build",
    "--debug",
    "--bundles",
    "app",
    "--",
    "--features",
    "automation",
  ]);
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
  assert.deepEqual(invocation.tauriExecArgs, [
    "-C",
    "apps/desktop",
    "exec",
    "tauri",
    "dev",
    "--no-watch",
  ]);
});
