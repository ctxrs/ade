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
  buildBazelPlatformArgs,
  buildBazelCommandContext,
  buildDesktopSyncEnv,
  parseBazelOutputPaths,
  parseArgs,
  renderDesktopSidecarEnv,
  shouldResolveAvfLinuxHelper,
} = require("./ctx_http_bazel.cjs");

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
      repoRoot: "/repo",
      targets: [
        "//core/crates/ctx-http:ctx",
        "//core/crates/ctx-mcp:ctx-mcp",
      ],
    },
  );
  assert.equal(outputs.get("//core/crates/ctx-http:ctx"), "/repo/bazel-out/k8-fastbuild/bin/core/crates/ctx-http/ctx");
  assert.equal(outputs.get("//core/crates/ctx-mcp:ctx-mcp"), "/repo/bazel-out/k8-fastbuild/bin/core/crates/ctx-mcp/ctx-mcp");
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
