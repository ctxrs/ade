#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");

const { createPrepSteps, parseArgs } = require("./desktop_prepare.cjs");

test("desktop_prepare rejects invalid modes", () => {
  const exit = process.exit;
  const error = console.error;
  const events = [];
  try {
    process.exit = (code) => {
      throw new Error(`exit:${code}`);
    };
    console.error = (message) => {
      events.push(String(message));
    };
    assert.throws(() => parseArgs(["node", "desktop_prepare", "--mode", "prod"]), /exit:1/);
    assert.match(events.join("\n"), /invalid --mode 'prod'/);
  } finally {
    process.exit = exit;
    console.error = error;
  }
});

test("desktop_prepare release mode builds web, checks versions, and syncs release resources", () => {
  const steps = createPrepSteps({
    mode: "release-build",
    cargoTargetDir: "/tmp/cargo-target",
    desktopVersion: "0.22.0",
    platform: "darwin",
    arch: "arm64",
  });

  assert.equal(steps[0].command, "node");
  assert.deepEqual(steps[0].args, ["scripts/desktop_check_versions.cjs"]);
  assert.deepEqual(steps[1].args, ["build", "-p", "ctx-http", "-p", "ctx-mcp", "--release"]);
  assert.deepEqual(steps[2].args, [
    "build",
    "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml",
    "--bin",
    "ctx-avf-linux-helper",
    "--release",
  ]);
  assert.equal(steps[2].env.CTX_DESKTOP_SKIP_TAURI_BUILD, "1");
  assert.deepEqual(steps[3].args, ["-C", "apps/web", "exec", "vite", "build"]);
  assert.equal(steps[3].env.VITE_CTX_APP_VERSION, "0.22.0");
  assert.deepEqual(steps[4].args, [
    "scripts/prepare_avf_linux_guest_runtime.sh",
    "--output-dir",
    "/tmp/cargo-target/desktop-avf-linux-guest-runtime",
    "--arch",
    "arm64",
    "--force",
  ]);
  assert.deepEqual(steps[5].args, ["scripts/desktop_sync_resources.cjs", "--profile", "release"]);
  assert.equal(steps[5].env.CTX_DESKTOP_SYNC_BUNDLES, "1");
  assert.equal(
    steps[5].env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR,
    "/tmp/cargo-target/desktop-avf-linux-guest-runtime",
  );
});

test("desktop_prepare dev mode skips web build and version checks", () => {
  const steps = createPrepSteps({
    mode: "dev",
    cargoTargetDir: "/tmp/cargo-target",
    desktopVersion: "0.22.0",
    platform: "linux",
    syncBundles: "1",
  });

  assert.equal(steps.length, 2);
  assert.deepEqual(steps[0].args, ["build", "-p", "ctx-http", "-p", "ctx-mcp"]);
  assert.deepEqual(steps[1].args, ["scripts/desktop_sync_resources.cjs", "--profile", "debug"]);
  assert.equal(steps[1].env.CTX_DESKTOP_SYNC_BUNDLES, "1");
});

test("desktop_prepare skips AVF guest runtime prep outside darwin arm64 release bundling", () => {
  const steps = createPrepSteps({
    mode: "release-build",
    cargoTargetDir: "/tmp/cargo-target",
    desktopVersion: "0.22.0",
    platform: "darwin",
    arch: "x64",
  });

  assert.equal(
    steps.some((step) => step.args.includes("scripts/prepare_avf_linux_guest_runtime.sh")),
    false,
  );
  assert.equal(
    Object.prototype.hasOwnProperty.call(steps.at(-1).env, "CTX_AVF_LINUX_GUEST_RUNTIME_DIR"),
    false,
  );
});

test("desktop_prepare skips AVF guest runtime prep when managed AVF metadata is allowed to satisfy prep", () => {
  const previous = process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD;
  process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD = "1";
  try {
    const steps = createPrepSteps({
      mode: "release-build",
      cargoTargetDir: "/tmp/cargo-target",
      desktopVersion: "0.22.0",
      platform: "darwin",
      arch: "arm64",
    });

    assert.equal(
      steps.some((step) => step.args.includes("scripts/prepare_avf_linux_guest_runtime.sh")),
      false,
    );
    assert.equal(
      Object.prototype.hasOwnProperty.call(steps.at(-1).env, "CTX_AVF_LINUX_GUEST_RUNTIME_DIR"),
      false,
    );
    assert.equal(steps.at(-1).env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD, "1");
  } finally {
    if (previous === undefined) {
      delete process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD;
    } else {
      process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD = previous;
    }
  }
});
