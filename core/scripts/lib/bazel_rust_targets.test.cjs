const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const {
  getAllCtxHttpSuiteCheckinFanoutTargets,
  getCtxHttpSuiteTargets,
} = require("./ctx_http_suites.cjs");
const { buildCheckinBuildkiteExecutionPlan } = require("./test_taxonomy/execution.cjs");
const { WEB_LINUX_RBE_SAFE_BAZEL_TEST_TARGETS } = require("./web_smoke_bazel_targets.cjs");

const {
  buildLinuxRbeSafeBazelTestTargets,
  getBazelBuildTargetsForCrates,
  getBazelClippyTargetsForCrates,
  getBazelCoveredCrates,
  getBazelTestTargetsForCrates,
  LINUX_RBE_SAFE_BAZEL_CLIPPY_TARGETS,
  LINUX_RBE_UNSAFE_BAZEL_TEST_TARGETS,
  partitionBazelTargetsForLinuxRbe,
} = require("./bazel_rust_targets.cjs");

const repoRoot = path.resolve(__dirname, "..", "..", "..");

function sortUnique(values) {
  return [...new Set(values)].sort();
}

test("Bazel-covered crates stay on the intended explicit Rust slice", () => {
  assert.deepEqual(getBazelCoveredCrates(), [
    "codex-crp",
    "ctx-avf-linux-guest-agent",
    "ctx-avf-linux-runtime",
    "ctx-bundled-assets",
    "ctx-client",
    "ctx-core",
    "ctx-crp-protocol",
    "ctx-daemon",
    "ctx-desktop-ipc",
    "ctx-docs-mirror",
    "ctx-egress-proxy",
    "ctx-events",
    "ctx-execution-runtime",
    "ctx-fs",
    "ctx-harness-runtime",
    "ctx-harness-setup",
    "ctx-harness-sources",
    "ctx-http",
    "ctx-http-auth",
    "ctx-http-test-support",
    "ctx-linux-sandbox-runtime",
    "ctx-llm-relay-authority",
    "ctx-llm-relay-contract",
    "ctx-load-test",
    "ctx-managed-installs",
    "ctx-mcp",
    "ctx-mcp-auth",
    "ctx-mcp-command",
    "ctx-merge-queue",
    "ctx-mobile-access-service",
    "ctx-observability",
    "ctx-org-policy",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-provider-install",
    "ctx-provider-matrix",
    "ctx-provider-runtime",
    "ctx-providers",
    "ctx-repo-onboarding-service",
    "ctx-resource-utilization",
    "ctx-route-contracts",
    "ctx-run-archive-service",
    "ctx-run-scheduler",
    "ctx-runtime-assets",
    "ctx-sandbox-container-runtime",
    "ctx-sandbox-contract",
    "ctx-sandbox-materialization",
    "ctx-session-artifacts",
    "ctx-session-message-service",
    "ctx-session-runtime",
    "ctx-session-service",
    "ctx-session-title-service",
    "ctx-session-tools",
    "ctx-session-vcs-service",
    "ctx-settings-model",
    "ctx-settings-service",
    "ctx-storage-admission",
    "ctx-store",
    "ctx-subagent-service",
    "ctx-task-service",
    "ctx-transport-runtime",
    "ctx-tunnel-control-plane",
    "ctx-tunnel-relay",
    "ctx-tunnel-router",
    "ctx-tunnel-store",
    "ctx-update-service",
    "ctx-workspace-active-snapshot",
    "ctx-workspace-attachments",
    "ctx-workspace-config",
    "ctx-workspace-container",
    "ctx-workspace-runtime",
    "ctx-workspace-stream-service",
    "ctx-worktree-bootstrap-service",
    "ctx-worktree-data-plane",
    "ctx-worktree-vcs-service",
  ]);
});

test("Bazel test target mapping expands per-crate tests deterministically", () => {
  assert.deepEqual(
    getBazelTestTargetsForCrates([
      "codex-crp",
      "ctx-avf-linux-guest-agent",
      "ctx-avf-linux-runtime",
      "ctx-client",
      "ctx-crp-protocol",
      "ctx-desktop-ipc",
      "ctx-docs-mirror",
      "ctx-egress-proxy",
      "ctx-execution-runtime",
      "ctx-core",
      "ctx-events",
      "ctx-fs",
      "ctx-http",
      "ctx-http-test-support",
      "ctx-llm-relay-authority",
      "ctx-llm-relay-contract",
      "ctx-load-test",
      "ctx-managed-installs",
      "ctx-mcp",
      "ctx-merge-queue",
      "ctx-org-policy",
      "ctx-provider-runtime",
      "ctx-repo-onboarding-service",
      "ctx-route-contracts",
      "ctx-run-archive-service",
      "ctx-run-scheduler",
      "ctx-session-artifacts",
      "ctx-session-message-service",
      "ctx-session-runtime",
      "ctx-session-title-service",
      "ctx-subagent-service",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-tunnel-control-plane",
      "ctx-tunnel-relay",
      "ctx-tunnel-router",
      "ctx-tunnel-store",
      "ctx-workspace-config",
      "ctx-core",
    ]),
    sortUnique([
      "//core/crates/codex-crp:unit_tests",
      "//core/crates/ctx-avf-linux-guest-agent:unit_tests",
      "//core/crates/ctx-avf-linux-runtime:helper_path_test_support",
      "//core/crates/ctx-avf-linux-runtime:unit_tests",
      "//core/crates/ctx-avf-linux-runtime:unit_tests_avf_linux_vm",
      "//core/crates/ctx-client:unit_tests",
      "//core/crates/ctx-core:unit_tests",
      "//core/crates/ctx-core:workspace_payload_corpus",
      "//core/crates/ctx-crp-protocol:unit_tests",
      "//core/crates/ctx-desktop-ipc:typescript_check_test",
      "//core/crates/ctx-docs-mirror:unit_tests",
      "//core/crates/ctx-egress-proxy:unit_tests",
      "//core/crates/ctx-events:unit_tests",
      "//core/crates/ctx-execution-runtime:unit_tests",
      "//core/crates/ctx-fs:unit_tests",
      ...getCtxHttpSuiteTargets("all"),
      "//core/crates/ctx-http-test-support:unit_tests",
      "//core/crates/ctx-llm-relay-authority:unit_tests",
      "//core/crates/ctx-llm-relay-contract:unit_tests",
      "//core/crates/ctx-managed-installs:unit_tests",
      "//core/crates/ctx-mcp:mcp_contracts",
      "//core/crates/ctx-mcp:subagent_tools",
      "//core/crates/ctx-merge-queue:unit_tests",
      "//core/crates/ctx-org-policy:unit_tests",
      "//core/crates/ctx-provider-runtime:unit_tests",
      "//core/crates/ctx-repo-onboarding-service:unit_tests",
      "//core/crates/ctx-route-contracts:unit_tests",
      "//core/crates/ctx-run-archive-service:unit_tests",
      "//core/crates/ctx-run-scheduler:unit_tests",
      "//core/crates/ctx-session-artifacts:unit_tests",
      "//core/crates/ctx-session-message-service:unit_tests",
      "//core/crates/ctx-session-runtime:unit_tests",
      "//core/crates/ctx-session-title-service:unit_tests",
      "//core/crates/ctx-subagent-service:unit_tests",
      "//core/crates/ctx-store:sqlite_hardening",
      "//core/crates/ctx-store:unit_tests",
      "//core/crates/ctx-transport-runtime:unit_tests",
      "//core/crates/ctx-tunnel-control-plane:unit_tests",
      "//core/crates/ctx-tunnel-relay:unit_tests",
      "//core/crates/ctx-tunnel-router:unit_tests",
      "//core/crates/ctx-tunnel-store:unit_tests",
      "//core/crates/ctx-workspace-config:unit_tests",
      "//core/tools/load-test:unit_tests",
    ]),
  );
});

test("Bazel build target mapping expands per-crate libraries deterministically", () => {
  assert.deepEqual(
    getBazelBuildTargetsForCrates([
      "ctx-avf-linux-helper",
      "codex-crp",
      "ctx-avf-linux-guest-agent",
      "ctx-avf-linux-runtime",
      "ctx-client",
      "ctx-crp-protocol",
      "ctx-docs-mirror",
      "ctx-provider-auth-import",
      "ctx-bundled-assets",
      "ctx-desktop-ipc",
      "ctx-egress-proxy",
      "ctx-execution-runtime",
      "ctx-events",
      "ctx-harness-setup",
      "ctx-http",
      "ctx-http-test-support",
      "ctx-load-test",
      "ctx-llm-relay-authority",
      "ctx-llm-relay-contract",
      "ctx-mcp",
      "ctx-merge-queue",
      "ctx-managed-installs",
      "ctx-org-policy",
      "ctx-provider-runtime",
      "ctx-repo-onboarding-service",
      "ctx-provider-install",
      "ctx-route-contracts",
      "ctx-runtime-assets",
      "ctx-sandbox-materialization",
      "ctx-session-artifacts",
      "ctx-session-message-service",
      "ctx-session-runtime",
      "ctx-session-title-service",
      "ctx-session-tools",
      "ctx-settings-model",
      "ctx-settings-service",
      "ctx-storage-admission",
      "ctx-tunnel-control-plane",
      "ctx-tunnel-relay",
      "ctx-tunnel-router",
      "ctx-tunnel-store",
      "ctx-transport-runtime",
      "ctx-workspace-active-snapshot",
      "ctx-workspace-runtime",
      "ctx-worktree-data-plane",
      "ctx-provider-matrix",
      "ctx-providers",
      "ctx-route-contracts",
      "ctx-run-archive-service",
      "ctx-run-scheduler",
      "ctx-session-artifacts",
      "ctx-session-message-service",
      "ctx-session-runtime",
      "ctx-session-title-service",
      "ctx-subagent-service",
      "ctx-core",
      "ctx-fs",
      "ctx-store",
      "ctx-workspace-config",
    ]),
    [
      "//core/apps/desktop/src-tauri/src:ctx-avf-linux-helper",
      "//core/crates/codex-crp:codex-crp",
      "//core/crates/ctx-avf-linux-guest-agent:ctx-avf-linux-guest-agent",
      "//core/crates/ctx-avf-linux-runtime:lib",
      "//core/crates/ctx-bundled-assets:lib",
      "//core/crates/ctx-client:lib",
      "//core/crates/ctx-core:lib",
      "//core/crates/ctx-crp-protocol:lib",
      "//core/crates/ctx-desktop-ipc:lib",
      "//core/crates/ctx-docs-mirror:ctx-docs-mirror",
      "//core/crates/ctx-egress-proxy:ctx-egress-proxy",
      "//core/crates/ctx-events:lib",
      "//core/crates/ctx-execution-runtime:lib",
      "//core/crates/ctx-fs:lib",
      "//core/crates/ctx-harness-setup:lib",
      "//core/crates/ctx-http-test-support:lib",
      "//core/crates/ctx-http:ctx",
      "//core/crates/ctx-http:lib",
      "//core/crates/ctx-llm-relay-authority:lib",
      "//core/crates/ctx-llm-relay-contract:lib",
      "//core/crates/ctx-managed-installs:lib",
      "//core/crates/ctx-mcp:ctx-mcp",
      "//core/crates/ctx-merge-queue:lib",
      "//core/crates/ctx-org-policy:lib",
      "//core/crates/ctx-provider-auth-import:lib",
      "//core/crates/ctx-provider-install:lib",
      "//core/crates/ctx-provider-matrix:lib",
      "//core/crates/ctx-provider-runtime:lib",
      "//core/crates/ctx-providers:lib",
      "//core/crates/ctx-repo-onboarding-service:lib",
      "//core/crates/ctx-route-contracts:lib",
      "//core/crates/ctx-run-archive-service:lib",
      "//core/crates/ctx-run-scheduler:lib",
      "//core/crates/ctx-runtime-assets:lib",
      "//core/crates/ctx-sandbox-materialization:lib",
      "//core/crates/ctx-session-artifacts:lib",
      "//core/crates/ctx-session-message-service:lib",
      "//core/crates/ctx-session-runtime:lib",
      "//core/crates/ctx-session-title-service:lib",
      "//core/crates/ctx-session-tools:lib",
      "//core/crates/ctx-settings-model:lib",
      "//core/crates/ctx-settings-service:lib",
      "//core/crates/ctx-storage-admission:lib",
      "//core/crates/ctx-store:lib",
      "//core/crates/ctx-subagent-service:lib",
      "//core/crates/ctx-transport-runtime:lib",
      "//core/crates/ctx-tunnel-control-plane:ctx-tunnel-control-plane",
      "//core/crates/ctx-tunnel-relay:ctx-tunnel-relay",
      "//core/crates/ctx-tunnel-router:ctx-tunnel-router",
      "//core/crates/ctx-tunnel-store:ctx-tunnel-cleanup",
      "//core/crates/ctx-tunnel-store:lib",
      "//core/crates/ctx-workspace-active-snapshot:lib",
      "//core/crates/ctx-workspace-config:lib",
      "//core/crates/ctx-workspace-runtime:lib",
      "//core/crates/ctx-worktree-data-plane:lib",
      "//core/tools/load-test:ctx-load-test",
    ],
  );
});

test("Bazel clippy target mapping expands crates to explicit build targets", () => {
  assert.deepEqual(
    getBazelClippyTargetsForCrates([
      "ctx-core",
      "ctx-http",
      "ctx-load-test",
      "ctx-core",
    ]),
    [
      "//core/crates/ctx-core:lib",
      "//core/crates/ctx-http:ctx",
      "//core/crates/ctx-http:lib",
      "//core/tools/load-test:ctx-load-test",
    ],
  );
});

test("Bazel clippy target mapping stays eligible for Linux RBE partitioning", () => {
  const { remoteTargets, localTargets } = partitionBazelTargetsForLinuxRbe(
    "build",
    getBazelClippyTargetsForCrates(["ctx-core", "ctx-http"]),
    { rustClippy: true },
  );
  assert.deepEqual(remoteTargets, [
    "//core/crates/ctx-core:lib",
    "//core/crates/ctx-http:ctx",
    "//core/crates/ctx-http:lib",
  ]);
  assert.deepEqual(localTargets, []);
});

test("ordinary Linux RBE build partitioning remains conservative for binary outputs", () => {
  const { remoteTargets, localTargets } = partitionBazelTargetsForLinuxRbe(
    "build",
    ["//core/crates/ctx-core:lib", "//core/crates/ctx-http:ctx"],
  );
  assert.deepEqual(remoteTargets, ["//core/crates/ctx-core:lib"]);
  assert.deepEqual(localTargets, ["//core/crates/ctx-http:ctx"]);
});

test("clippy Linux RBE partitioning covers explicit build targets including binaries", () => {
  assert.equal(LINUX_RBE_SAFE_BAZEL_CLIPPY_TARGETS.includes("//core/crates/ctx-http:ctx"), true);
  const { remoteTargets, localTargets } = partitionBazelTargetsForLinuxRbe(
    "build",
    ["//core/crates/ctx-core:lib", "//core/crates/ctx-http:ctx"],
    { rustClippy: true },
  );
  assert.deepEqual(remoteTargets, ["//core/crates/ctx-core:lib", "//core/crates/ctx-http:ctx"]);
  assert.deepEqual(localTargets, []);
});

test("flattened ctx-http checkin fanout targets are Linux RBE safe", () => {
  const fanoutTargets = getAllCtxHttpSuiteCheckinFanoutTargets();
  assert.ok(fanoutTargets.length > getCtxHttpSuiteTargets("all").length);
  const { localTargets } = partitionBazelTargetsForLinuxRbe("test", fanoutTargets);
  assert.deepEqual(localTargets, []);
  assert.equal(
    LINUX_RBE_UNSAFE_BAZEL_TEST_TARGETS.has("//core/crates/ctx-http:unit-tests-provider-and-settings"),
    true,
  );
});

test("checkin fans out ctx-http child targets for Buildkite", () => {
  const plan = buildCheckinBuildkiteExecutionPlan({ profileId: "checkin" });
  const allFanoutTargets = getAllCtxHttpSuiteCheckinFanoutTargets();
  const directBazelCommands = plan.commands
    .filter((command) =>
      command.startsWith("node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:")
    );
  const directBazelTargets = directBazelCommands
    .flatMap((command) => command.replace("node scripts/run_bazel_pilot.cjs test ", "").split(" "));
  const ctxHttpSuiteCommands = plan.commands.filter((command) =>
    command.startsWith("node scripts/ctx_http_suite_task.cjs --suite ")
  );

  assert.ok(directBazelTargets.length > getCtxHttpSuiteTargets("all").length);
  for (const target of directBazelTargets) {
    assert.ok(allFanoutTargets.includes(target), `${target} should be a known ctx-http fanout target`);
  }
  assert.equal(ctxHttpSuiteCommands.length, 0);
  assert.ok(directBazelTargets.includes("//core/crates/ctx-http:unit_tests_api"));
  assert.ok(directBazelTargets.includes("//core/crates/ctx-http:bin_tests_root_help"));
  assert.ok(directBazelCommands.some((command) =>
    command.includes("//core/crates/ctx-http:bin_tests_root_help")
    && command.includes("//core/crates/ctx-http:bin_tests_self_update_help")
  ));
});

test("checkin ctx-http fanout targets are Linux RBE-safe Bazel labels", () => {
  const plan = buildCheckinBuildkiteExecutionPlan({ profileId: "checkin" });
  const ctxHttpFanoutTargets = plan.commands
    .filter((command) =>
      command.startsWith("node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:")
    )
    .flatMap((command) => command.replace("node scripts/run_bazel_pilot.cjs test ", "").split(" "));
  assert.ok(ctxHttpFanoutTargets.length > getCtxHttpSuiteTargets("all").length);
  for (const target of ctxHttpFanoutTargets) {
    assert.ok(
      getAllCtxHttpSuiteCheckinFanoutTargets().includes(target),
      `${target} should be a known ctx-http fanout target`,
    );
  }

  const { localTargets } = partitionBazelTargetsForLinuxRbe("test", ctxHttpFanoutTargets);
  assert.deepEqual(localTargets, []);
});

test("checkin ctx-http fanout targets can be compiled by Linux RBE warmup builds", () => {
  const plan = buildCheckinBuildkiteExecutionPlan({ profileId: "checkin" });
  const ctxHttpFanoutTargets = plan.commands
    .filter((command) =>
      command.startsWith("node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:")
    )
    .flatMap((command) => command.replace("node scripts/run_bazel_pilot.cjs test ", "").split(" "));
  const { localTargets, remoteTargets } = partitionBazelTargetsForLinuxRbe("build", ctxHttpFanoutTargets);

  assert.ok(ctxHttpFanoutTargets.length > getCtxHttpSuiteTargets("all").length);
  assert.deepEqual(localTargets, []);
  assert.deepEqual(remoteTargets, sortUnique(ctxHttpFanoutTargets));
});

test("web checkin lint typecheck and unit Bazel targets are Linux RBE safe", () => {
  const packageJson = JSON.parse(fs.readFileSync(path.join(repoRoot, "core", "package.json"), "utf8"));
  const plan = buildCheckinBuildkiteExecutionPlan({ profileId: "checkin" });
  const webCheckinScripts = plan.commands
    .filter((command) => command.startsWith("pnpm bazel:web:") && !command.startsWith("pnpm bazel:web:e2e:"))
    .map((command) => command.replace(/^pnpm /u, ""));
  assert.ok(webCheckinScripts.length > 5);

  const targets = [];
  for (const scriptName of webCheckinScripts) {
    const script = String(packageJson.scripts[scriptName] || "");
    for (const match of script.matchAll(/node scripts\/run_bazel_pilot\.cjs test ([^&]+)/gu)) {
      targets.push(...match[1].trim().split(/\s+/u).filter(Boolean));
    }
  }

  assert.ok(targets.includes("//core/apps/web:typecheck"));
  assert.equal(WEB_LINUX_RBE_SAFE_BAZEL_TEST_TARGETS.includes("//core/apps/web/e2e:premerge_required"), false);
  const { localTargets } = partitionBazelTargetsForLinuxRbe("test", targets);
  assert.deepEqual(localTargets, []);
});

test("browser web e2e remains outside the first Linux RBE safe expansion", () => {
  const { remoteTargets, localTargets } = partitionBazelTargetsForLinuxRbe(
    "test",
    ["//core/apps/web/e2e:premerge_required"],
  );
  assert.deepEqual(remoteTargets, []);
  assert.deepEqual(localTargets, ["//core/apps/web/e2e:premerge_required"]);
});

test("all Bazel-covered Rust crates have clippy build targets", () => {
  for (const crateName of getBazelCoveredCrates()) {
    assert.notDeepEqual(
      getBazelClippyTargetsForCrates([crateName]),
      [],
      `${crateName} should map to at least one clippy package target`,
    );
  }
});

test("linux RBE keeps Darwin-incompatible guest-agent tests local", () => {
  assert.deepEqual(
    partitionBazelTargetsForLinuxRbe("test", [
      "//core/crates/ctx-avf-linux-guest-agent:unit_tests",
      "//core/crates/ctx-store:unit_tests",
    ]),
    {
      remoteTargets: ["//core/crates/ctx-store:unit_tests"],
      localTargets: ["//core/crates/ctx-avf-linux-guest-agent:unit_tests"],
    },
  );
});

test("linux RBE promotes web-smoke Bazel tests into the remote-safe partition", () => {
  assert.deepEqual(
    partitionBazelTargetsForLinuxRbe("test", [
      "//core/packages/session-supervisor-core:unit_tests",
      "//core/packages/session-thread-layout:unit_smoke",
      "//core/apps/web:unit_smoke",
    ]),
    {
      remoteTargets: [
        "//core/apps/web:unit_smoke",
        "//core/packages/session-supervisor-core:unit_tests",
        "//core/packages/session-thread-layout:unit_smoke",
      ],
      localTargets: [],
    },
  );
});

test("linux RBE unsafe targets keep final precedence over manual promotions", () => {
  const safeTargets = buildLinuxRbeSafeBazelTestTargets({
    unsafeTargets: new Set([
      "//core/apps/web:unit_smoke",
    ]),
  });

  assert.equal(safeTargets.includes("//core/apps/web:unit_smoke"), false);
  assert.equal(safeTargets.includes("//core/packages/session-supervisor-core:unit_tests"), true);
});
