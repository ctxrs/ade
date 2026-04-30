#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");

const {
  BAZEL_PILOT_SCRIPT_PATH,
  DEFAULT_PROFILE,
  DESKTOP_SIDECAR_TARGETS,
  PREPARE_DESKTOP_SIDECARS_COMMAND,
  PRINT_DESKTOP_SIDECAR_ENV_COMMAND,
  TARGET_SPECS,
  buildBazelIdentityActionEnvArgs,
  buildBazelPlatformArgs,
  buildBazelCommandContext,
  buildDesktopSidecarIdentityEnv,
  buildTargetsViaBazel,
  buildDesktopSyncEnv,
  parseBazelOutputPaths,
  parseArgs,
  renderDesktopSidecarEnv,
  resolveBazelExecutionRoot,
  resolveBazelOutputPaths,
  syncDesktopResources,
  shouldResolveAvfLinuxHelper,
} = require("./ctx_http_bazel.cjs");
const { HOST_HEAVY_BUDGET_KEY } = require("./lib/host_job_budget.cjs");

test("ctx_http_bazel parses the desktop sidecar command with a release default", () => {
  assert.deepEqual(parseArgs([]), {
    command: PREPARE_DESKTOP_SIDECARS_COMMAND,
    profile: DEFAULT_PROFILE,
    targetKey: "",
  });
  assert.deepEqual(parseArgs(["prepare-desktop-sidecars", "--profile", "debug"]), {
    command: PREPARE_DESKTOP_SIDECARS_COMMAND,
    profile: "debug",
    targetKey: "",
  });
  assert.deepEqual(parseArgs(["print-sidecar-env", "--target-key", "darwin-aarch64"]), {
    command: PRINT_DESKTOP_SIDECAR_ENV_COMMAND,
    profile: DEFAULT_PROFILE,
    targetKey: "darwin-aarch64",
  });
});

test("ctx_http_bazel rejects unsupported commands and profiles", () => {
  assert.throws(() => parseArgs(["build"]), /unsupported ctx-http Bazel command/);
  assert.throws(() => parseArgs(["prepare-desktop-sidecars", "--profile", "prod"]), /unsupported --profile/);
  assert.throws(() => parseArgs(["print-sidecar-env", "--target-key", "darwin-amd64"]), /unsupported ctx-http Bazel target/);
});

test("ctx_http_bazel sync env always injects Bazel-built sidecar paths", () => {
  assert.deepEqual(buildDesktopSyncEnv({
    env: { BASE: "1" },
    ctxBinPath: "/tmp/bazel-bin/ctx",
    ctxMcpBinPath: "/tmp/bazel-bin/ctx-mcp",
    avfLinuxHelperBinPath: "/tmp/bazel-bin/ctx-avf-linux-helper",
    desktopWebDist: "/tmp/web-dist",
    profile: "release",
  }), {
    BASE: "1",
    CTX_DESKTOP_CTX_BIN: "/tmp/bazel-bin/ctx",
    CTX_DESKTOP_CTX_MCP_BIN: "/tmp/bazel-bin/ctx-mcp",
    CTX_DESKTOP_AVF_LINUX_HELPER_BIN: "/tmp/bazel-bin/ctx-avf-linux-helper",
    CTX_DESKTOP_SYNC_BUNDLES: "0",
    CTX_DESKTOP_SYNC_PROFILE: "release",
    CTX_DESKTOP_WEB_DIST: "/tmp/web-dist",
  });
  assert.deepEqual(Object.keys(TARGET_SPECS), [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-aarch64",
    "linux-x86_64",
  ]);
  assert.deepEqual(DESKTOP_SIDECAR_TARGETS, {
    ctxBin: "//core/crates/ctx-http:ctx",
    ctxMcpBin: "//core/crates/ctx-mcp:ctx-mcp",
    avfLinuxHelperBin: "//core/apps/desktop/src-tauri/src:ctx-avf-linux-helper",
  });
});

test("ctx_http_bazel stamps release identity into sidecar build env", () => {
  const env = buildDesktopSidecarIdentityEnv({
    env: {
      RELEASE_CHANNEL: "canary",
      RELEASE_SOURCE_COMMIT: "a14656fea33485e21cbdc15a4b8254d020404fd3",
      RELEASE_VERSION: "0.62.0",
    },
    profile: "release",
  });
  assert.equal(env.CTX_RELEASE_EFFECTIVE_VERSION, "0.62.0");
  assert.equal(env.CTX_BUILD_ID, "a14656fea334");
  assert.equal(
    env.CTX_COMPATIBILITY_TOKEN,
    "artifact-a14656fea33485e21cbdc15a4b8254d020404fd3",
  );
  assert.equal(env.CTX_DEV_INSTANCE_ID, env.CTX_COMPATIBILITY_TOKEN);
});

test("ctx_http_bazel forwards release identity into Bazel action env", () => {
  const args = buildBazelIdentityActionEnvArgs({
    CTX_RELEASE_EFFECTIVE_VERSION: "0.62.22-preview.deadbeef",
    CTX_BUILD_ID: "deadbeef",
    CTX_COMPATIBILITY_TOKEN: "artifact-deadbeef",
    CTX_DEV_INSTANCE_ID: "artifact-deadbeef",
  });
  assert.deepEqual(args, [
    "--action_env=CTX_RELEASE_EFFECTIVE_VERSION=0.62.22-preview.deadbeef",
    "--action_env=CTX_BUILD_ID=deadbeef",
    "--action_env=CTX_COMPATIBILITY_TOKEN=artifact-deadbeef",
    "--action_env=CTX_DEV_INSTANCE_ID=artifact-deadbeef",
  ]);
});

test("ctx_http_bazel maps explicit target keys to Bazel platform labels", () => {
  assert.deepEqual(buildBazelPlatformArgs(""), []);
  assert.deepEqual(buildBazelPlatformArgs("darwin-aarch64"), ["--platforms=//tools/bazel/platforms:darwin_arm64"]);
  assert.deepEqual(buildBazelPlatformArgs("darwin-x86_64"), ["--platforms=//tools/bazel/platforms:darwin_x86_64"]);
  assert.deepEqual(buildBazelPlatformArgs("linux-aarch64"), ["--platforms=//tools/bazel/platforms:linux_arm64"]);
  assert.deepEqual(buildBazelPlatformArgs("linux-x86_64"), ["--platforms=//tools/bazel/platforms:linux_x86_64"]);
  assert.throws(() => buildBazelPlatformArgs("bogus"), /unsupported ctx-http Bazel target/);
});

test("ctx_http_bazel uses plain Bazel for direct builds when the repo pins the BuildBuddy wrapper", () => {
  const context = buildBazelCommandContext({});
  assert.equal(context.env.USE_BAZEL_VERSION, "9.0.1");
  assert.equal(context.bazelCommandArgs[0].startsWith("--disk_cache="), true);
  assert.equal(context.bazelCommandArgs[1].startsWith("--repository_cache="), true);
});

test("ctx_http_bazel preserves an explicit USE_BAZEL_VERSION override", () => {
  const context = buildBazelCommandContext({ USE_BAZEL_VERSION: "8.2.0" });
  assert.equal(context.env.USE_BAZEL_VERSION, "8.2.0");
});

test("ctx_http_bazel resolves multiple Bazel output paths from a single cquery payload", () => {
  const outputs = parseBazelOutputPaths(
    [
      "@@//core/crates/ctx-http:ctx|bazel-out/k8-fastbuild/bin/core/crates/ctx-http/ctx",
      "@@//core/crates/ctx-mcp:ctx-mcp|bazel-out/k8-fastbuild/bin/core/crates/ctx-mcp/ctx-mcp",
    ].join("\n"),
    {
      executionRoot: "/execroot",
      repoRoot: "/repo",
      targets: [
        "//core/crates/ctx-http:ctx",
        "//core/crates/ctx-mcp:ctx-mcp",
      ],
    },
  );
  assert.equal(outputs.get("//core/crates/ctx-http:ctx"), "/execroot/bazel-out/k8-fastbuild/bin/core/crates/ctx-http/ctx");
  assert.equal(outputs.get("//core/crates/ctx-mcp:ctx-mcp"), "/execroot/bazel-out/k8-fastbuild/bin/core/crates/ctx-mcp/ctx-mcp");
});

test("ctx_http_bazel rejects missing Bazel output lines", () => {
  assert.throws(
    () => parseBazelOutputPaths("//core/crates/ctx-http:ctx|bazel-out/bin/ctx", {
      repoRoot: "/repo",
      targets: ["//core/crates/ctx-http:ctx", "//core/crates/ctx-mcp:ctx-mcp"],
    }),
    /missing Bazel output for \/\/core\/crates\/ctx-mcp:ctx-mcp/,
  );
});

test("ctx_http_bazel renders shell-safe sidecar env exports", () => {
  assert.equal(
    renderDesktopSidecarEnv({
      ctxBinPath: "/tmp/bazel-bin/ctx",
      ctxMcpBinPath: "/tmp/bazel-bin/ctx-mcp",
      avfLinuxHelperBinPath: "/tmp/bazel-bin/ctx-avf-linux-helper",
    }),
    [
      "export CTX_DESKTOP_CTX_BIN='/tmp/bazel-bin/ctx'",
      "export CTX_DESKTOP_CTX_MCP_BIN='/tmp/bazel-bin/ctx-mcp'",
      "export CTX_DESKTOP_AVF_LINUX_HELPER_BIN='/tmp/bazel-bin/ctx-avf-linux-helper'",
    ].join("\n"),
  );
});

test("ctx_http_bazel only resolves the AVF helper on darwin targets", () => {
  assert.equal(shouldResolveAvfLinuxHelper("", "darwin"), true);
  assert.equal(shouldResolveAvfLinuxHelper("", "linux"), false);
  assert.equal(shouldResolveAvfLinuxHelper("darwin-aarch64", "linux"), true);
  assert.equal(shouldResolveAvfLinuxHelper("darwin-x86_64", "linux"), true);
  assert.equal(shouldResolveAvfLinuxHelper("linux-x86_64", "darwin"), false);
});

test("ctx_http_bazel keeps the Bazel pilot script path repo-root relative", () => {
  assert.equal(BAZEL_PILOT_SCRIPT_PATH, "core/scripts/run_bazel_pilot.cjs");
});

test("ctx_http_bazel budgets direct Bazel builds under host-heavy", () => {
  const budgetCalls = [];
  const spawnCalls = [];

  buildTargetsViaBazel(["//core/crates/ctx-http:ctx"], {
    env: {
      ...process.env,
      CTX_RELEASE_EFFECTIVE_VERSION: "0.62.22-preview.deadbeef",
      CTX_BUILD_ID: "deadbeef",
      CTX_COMPATIBILITY_TOKEN: "artifact-deadbeef",
      CTX_DEV_INSTANCE_ID: "artifact-deadbeef",
    },
    quietStdout: true,
    spawnSyncImpl: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
    withHostJobBudgetImpl: (options, fn) => {
      budgetCalls.push(options);
      return fn();
    },
  });

  assert.equal(budgetCalls.length, 1);
  assert.equal(budgetCalls[0].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(spawnCalls.length, 1);
  assert.equal(spawnCalls[0].args.includes("--action_env=CTX_RELEASE_EFFECTIVE_VERSION=0.62.22-preview.deadbeef"), true);
  assert.equal(spawnCalls[0].args.includes("--action_env=CTX_BUILD_ID=deadbeef"), true);
});

test("ctx_http_bazel budgets direct Bazel cquery lookups under host-heavy", () => {
  const budgetCalls = [];
  const spawnCalls = [];

  const outputs = resolveBazelOutputPaths(["//core/crates/ctx-http:ctx"], {
    env: process.env,
    spawnSyncImpl: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      if (args.includes("info")) {
        return { status: 0, stdout: "/execroot\n" };
      }
      return {
        status: 0,
        stdout: "@@//core/crates/ctx-http:ctx|bazel-out/k8-fastbuild/bin/core/crates/ctx-http/ctx\n",
      };
    },
    withHostJobBudgetImpl: (options, fn) => {
      budgetCalls.push(options);
      return fn();
    },
  });

  assert.equal(budgetCalls.length, 2);
  assert.equal(budgetCalls[0].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(budgetCalls[0].command, "bazel info execution_root");
  assert.equal(budgetCalls[1].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.match(budgetCalls[1].command, /^bazel cquery /);
  assert.equal(spawnCalls.length, 2);
  assert.equal(
    outputs.get("//core/crates/ctx-http:ctx"),
    "/execroot/bazel-out/k8-fastbuild/bin/core/crates/ctx-http/ctx",
  );
});

test("ctx_http_bazel rejects empty Bazel execution root", () => {
  assert.throws(
    () => resolveBazelExecutionRoot({
      env: process.env,
      spawnSyncImpl: () => ({ status: 0, stdout: "\n" }),
      withHostJobBudgetImpl: (_options, fn) => fn(),
    }),
    /execution_root returned an empty path/,
  );
});

test("ctx_http_bazel budgets desktop resource sync under host-heavy", () => {
  const budgetCalls = [];
  const runCalls = [];

  syncDesktopResources({
    env: {
      CTX_DESKTOP_WEB_DIST: "/tmp/web-dist",
    },
    profile: "release",
    ctxBinPath: "/tmp/bazel-bin/ctx",
    ctxMcpBinPath: "/tmp/bazel-bin/ctx-mcp",
    runCheckedImpl: (...args) => {
      runCalls.push(args);
      return { status: 0 };
    },
    withHostJobBudgetImpl: (options, fn) => {
      budgetCalls.push(options);
      return fn();
    },
  });

  assert.equal(budgetCalls.length, 1);
  assert.equal(budgetCalls[0].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(runCalls.length, 1);
});
