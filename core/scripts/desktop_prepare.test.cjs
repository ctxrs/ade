#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");

const { DESKTOP_PREPARE_BUDGET_KEY, HOST_HEAVY_BUDGET_KEY } = require("./lib/host_job_budget.cjs");
const { createPrepSteps, main, parseArgs, resolveDesktopWebDist } = require("./desktop_prepare.cjs");

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
    desktopWebDist: "/tmp/web-dist",
    desktopVersion: "0.22.0",
    platform: "darwin",
    arch: "arm64",
    prepEnv: {
      CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
      CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
      CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-out/ctx-avf-linux-helper",
    },
  });

  assert.equal(steps[0].command, "node");
  assert.deepEqual(steps[0].args, ["scripts/desktop_check_versions.cjs"]);
  assert.deepEqual(steps[1].args, ["../scripts/ensure_macos_avf_build_tools.sh"]);
  assert.deepEqual(steps[2].args, [
    "scripts/prepare_avf_linux_guest_runtime.sh",
    "--output-dir",
    "/tmp/cargo-target/desktop-avf-linux-guest-runtime",
    "--arch",
    "arm64",
    "--force",
  ]);
  assert.deepEqual(steps[3].args, ["scripts/desktop_sync_resources.cjs", "--profile", "release"]);
  assert.equal(steps[3].env.CTX_DESKTOP_SYNC_BUNDLES, "1");
  assert.equal(
    steps[3].env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR,
    "/tmp/cargo-target/desktop-avf-linux-guest-runtime",
  );
  assert.equal(steps[3].env.CTX_DESKTOP_WEB_DIST, "/tmp/web-dist");
});

test("desktop_prepare dev mode skips web build and version checks", () => {
  const steps = createPrepSteps({
    mode: "dev",
    cargoTargetDir: "/tmp/cargo-target",
    desktopVersion: "0.22.0",
    platform: "linux",
    syncBundles: "1",
    prepEnv: {
      CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
      CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
    },
  });

  assert.equal(steps.length, 1);
  assert.deepEqual(steps[0].args, ["scripts/desktop_sync_resources.cjs", "--profile", "debug"]);
  assert.equal(steps[0].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(steps[0].env.CTX_DESKTOP_SYNC_BUNDLES, "1");
});

test("desktop_prepare build modes prefer an explicit CTX_DESKTOP_WEB_DIST", () => {
  const desktopWebDist = resolveDesktopWebDist({
    mode: "release-build",
    coreRoot: "/repo/core",
    prepEnv: {
      CTX_DESKTOP_WEB_DIST: "apps/web/dist",
    },
    desktopVersion: "0.22.0",
    ensureWebDistArtifactImpl: () => {
      throw new Error("explicit CTX_DESKTOP_WEB_DIST should bypass Bazel web dist preparation");
    },
  });

  assert.equal(desktopWebDist, "/repo/core/apps/web/dist");
});

test("desktop_prepare build modes fall back to cached Bazel web dist when no override is set", () => {
  let invocation = null;
  const desktopWebDist = resolveDesktopWebDist({
    mode: "debug-build",
    coreRoot: "/repo/core",
    prepEnv: {},
    desktopVersion: "0.22.0",
    ensureWebDistArtifactImpl: (options) => {
      invocation = options;
      return { distDir: "/tmp/web-dist" };
    },
  });

  assert.equal(desktopWebDist, "/tmp/web-dist");
  assert.deepEqual(invocation, {
    coreRoot: "/repo/core",
    env: {},
    appVersion: "0.22.0",
    variant: "desktop-debug-build",
  });
});

test("desktop_prepare forwards explicit Bazel sidecar paths into desktop sync", () => {
  const steps = createPrepSteps({
    mode: "release-build",
    cargoTargetDir: "/tmp/cargo-target",
    desktopWebDist: "/tmp/web-dist",
    desktopVersion: "0.22.0",
    platform: "darwin",
    arch: "arm64",
    prepEnv: {
      CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
      CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
      CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-out/ctx-avf-linux-helper",
    },
  });

  assert.equal(steps.at(-1).env.CTX_DESKTOP_CTX_BIN, "/tmp/bazel-out/ctx");
  assert.equal(steps.at(-1).env.CTX_DESKTOP_CTX_MCP_BIN, "/tmp/bazel-out/ctx-mcp");
  assert.equal(steps.at(-1).env.CTX_DESKTOP_AVF_LINUX_HELPER_BIN, "/tmp/bazel-out/ctx-avf-linux-helper");
});

test("desktop_prepare rejects missing Bazel sidecar env for linux", () => {
  assert.throws(
    () => createPrepSteps({
      mode: "dev",
      cargoTargetDir: "/tmp/cargo-target",
      desktopVersion: "0.22.0",
      platform: "linux",
    }),
    /requires Bazel-resolved CTX_DESKTOP_CTX_BIN and CTX_DESKTOP_CTX_MCP_BIN/,
  );
});

test("desktop_prepare rejects missing Bazel AVF helper env on darwin", () => {
  assert.throws(
    () => createPrepSteps({
      mode: "release-build",
      cargoTargetDir: "/tmp/cargo-target",
      desktopWebDist: "/tmp/web-dist",
      desktopVersion: "0.22.0",
      platform: "darwin",
      arch: "arm64",
      prepEnv: {
        CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
        CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
      },
    }),
    /requires Bazel-resolved CTX_DESKTOP_AVF_LINUX_HELPER_BIN on darwin/,
  );
});

test("desktop_prepare skips AVF guest runtime prep outside darwin arm64 release bundling", () => {
  const steps = createPrepSteps({
    mode: "release-build",
    cargoTargetDir: "/tmp/cargo-target",
    desktopWebDist: "/tmp/web-dist",
    desktopVersion: "0.22.0",
    platform: "darwin",
    arch: "x64",
    prepEnv: {
      CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
      CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
      CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-out/ctx-avf-linux-helper",
    },
  });

  assert.equal(
    steps.some((step) => step.args.includes("scripts/prepare_avf_linux_guest_runtime.sh")),
    false,
  );
  assert.equal(
    steps.some((step) => step.args.includes("../scripts/ensure_macos_avf_build_tools.sh")),
    true,
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
      desktopWebDist: "/tmp/web-dist",
      desktopVersion: "0.22.0",
      platform: "darwin",
      arch: "arm64",
      prepEnv: {
        CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
        CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
        CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-out/ctx-avf-linux-helper",
      },
    });

    assert.equal(
      steps.some((step) => step.args.includes("scripts/prepare_avf_linux_guest_runtime.sh")),
      false,
    );
    assert.equal(
      steps.some((step) => step.args.includes("../scripts/ensure_macos_avf_build_tools.sh")),
      true,
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

test("desktop_prepare skips AVF tool bootstrap in dev mode", () => {
  const steps = createPrepSteps({
    mode: "dev",
    cargoTargetDir: "/tmp/cargo-target",
    desktopVersion: "0.22.0",
    platform: "darwin",
    arch: "arm64",
    prepEnv: {
      CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
      CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
      CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-out/ctx-avf-linux-helper",
    },
  });

  assert.equal(
    steps.some((step) => step.args.includes("../scripts/ensure_macos_avf_build_tools.sh")),
    false,
  );
});

test("desktop_prepare marks expensive materialization steps with host-heavy budget", () => {
  const steps = createPrepSteps({
    mode: "release-build",
    cargoTargetDir: "/tmp/cargo-target",
    desktopWebDist: "/tmp/web-dist",
    desktopVersion: "0.22.0",
    platform: "darwin",
    arch: "arm64",
    prepEnv: {
      CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
      CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
      CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-out/ctx-avf-linux-helper",
    },
  });

  assert.equal(steps[0].budgetKey, undefined);
  assert.equal(steps[1].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(steps[2].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(steps[3].budgetKey, HOST_HEAVY_BUDGET_KEY);
});

test("desktop_prepare serializes the overall prep loop and nests host-heavy materialization", () => {
  const budgetCalls = [];
  const spawnCalls = [];

  main(["node", "desktop_prepare", "--mode", "dev"], {
    buildCtxCacheEnvImpl: () => ({
      cargoTargetDir: "/tmp/cargo-target",
      env: {
        CTX_DESKTOP_CTX_BIN: "/tmp/bazel-out/ctx",
        CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-out/ctx-mcp",
        CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-out/ctx-avf-linux-helper",
      },
    }),
    readDesktopVersionImpl: () => "0.22.0",
    spawnSyncImpl: (command, args) => {
      spawnCalls.push([command, args]);
      return { status: 0 };
    },
    withHostJobBudgetImpl: (options, fn) => {
      budgetCalls.push(options);
      return fn();
    },
  });

  assert.deepEqual(
    budgetCalls.map((call) => call.budgetKey),
    [DESKTOP_PREPARE_BUDGET_KEY, HOST_HEAVY_BUDGET_KEY],
  );
  assert.equal(spawnCalls.length, 1);
});
