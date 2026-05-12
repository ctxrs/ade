const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  MANUAL_ONLY_RUST_CRATES,
  buildGeneratedPackageScripts,
  buildWorkspaceGraph,
  collectChangedCrates,
  expandReverseDependencies,
  getCtxHttpSuiteTaskName,
  getPackageScriptNamesForCrates,
  getPackageTaskCommandsForCrates,
} = require("./rust_workspace_graph.cjs");

const coreRoot = path.resolve(__dirname, "../..");

test("workspace graph maps extracted leaf crate paths to crate names", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const crates = collectChangedCrates(graph, [
    "core/crates/ctx-bundled-assets/src/lib.rs",
    "core/crates/ctx-harness-setup/src/lib.rs",
    "core/crates/ctx-provider-accounts/src/lib.rs",
    "core/crates/ctx-provider-auth-import/src/lib.rs",
    "core/crates/ctx-harness-sources/src/lib.rs",
    "core/crates/ctx-mcp-command/src/lib.rs",
    "core/crates/ctx-http-test-support/src/lib.rs",
    "core/crates/ctx-mcp/src/lib.rs",
    "core/crates/ctx-runtime-assets/src/lib.rs",
    "core/crates/ctx-sandbox-contract/src/lib.rs",
    "core/crates/ctx-storage-admission/src/lib.rs",
    "core/crates/ctx-worktree-data-plane/src/lib.rs",
    "core/crates/ctx-workspace-active-snapshot/src/lib.rs",
  ]);

  assert.deepEqual(crates, [
    "ctx-bundled-assets",
    "ctx-harness-setup",
    "ctx-harness-sources",
    "ctx-http-test-support",
    "ctx-mcp",
    "ctx-mcp-command",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-runtime-assets",
    "ctx-sandbox-contract",
    "ctx-storage-admission",
    "ctx-workspace-active-snapshot",
    "ctx-worktree-data-plane",
  ]);
});

test("workspace graph expands reverse dependencies through extracted ctx-http leaves", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const impacted = expandReverseDependencies(graph, ["ctx-provider-accounts"]);

  assert.equal(impacted.includes("ctx-provider-accounts"), true);
  assert.equal(impacted.includes("ctx-harness-sources"), true);
  assert.equal(impacted.includes("ctx-provider-auth-import"), true);
  assert.equal(impacted.includes("ctx-http"), true);

  const workspaceSnapshotImpacted = expandReverseDependencies(graph, [
    "ctx-workspace-active-snapshot",
  ]);
  assert.equal(workspaceSnapshotImpacted.includes("ctx-workspace-active-snapshot"), true);
  assert.equal(workspaceSnapshotImpacted.includes("ctx-http"), true);
});

test("generated package scripts include per-crate and ctx-http suite tasks", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const scripts = buildGeneratedPackageScripts(graph);

  assert.equal(scripts["rust:package-scripts:sync"], "node scripts/sync_rust_package_scripts.cjs");
  assert.equal(scripts["rust:package-scripts:check"], "node scripts/sync_rust_package_scripts.cjs --check");
  assert.equal(typeof scripts["rust:crate:clippy:ctx-http"], "string");
  assert.equal(scripts["rust:crate:test:ctx-http"], "node scripts/ctx_http_suite_task.cjs --suite all");
  assert.equal(typeof scripts["rust:crate:nextest:ctx-http"], "string");
  assert.equal(typeof scripts["rust:ctx-http:test:workspace-stream"], "string");
});

test("ctx-http test expansion returns explicit suite task names", () => {
  const taskNames = getPackageScriptNamesForCrates(
    ["ctx-http"],
    ["test"],
  );

  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("bin-tests")), true);
  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("workspace-stream")), true);
  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("provider-runtime-simulated")), true);
  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("provider-runtime-live")), true);
  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("sandbox-runtime-simulated")), true);
  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("subagents-control")), true);
});

test("package task command lookup preserves generated command special cases", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const commands = getPackageTaskCommandsForCrates(graph, ["ctx-core", "ctx-http"], ["test"]);

  assert.equal(
    commands.some((task) => task.command === "node scripts/rust_crate_task.cjs --crate ctx-core --task test"),
    true,
  );
  assert.equal(
    commands.some((task) => task.command === "node scripts/ctx_http_suite_task.cjs --suite bin-tests"),
    true,
  );
  assert.equal(
    commands.some((task) => task.command === "node scripts/ctx_http_suite_task.cjs --suite workspace-stream"),
    true,
  );
});

test("manual-only Rust crates stay out of generated CI task surfaces", () => {
  assert.equal(MANUAL_ONLY_RUST_CRATES.has("ctx-worker-gateway"), true);

  const graph = buildWorkspaceGraph(coreRoot);
  const scripts = buildGeneratedPackageScripts(graph);

  assert.equal(scripts["rust:crate:clippy:ctx-worker-gateway"], undefined);
  assert.equal(scripts["rust:crate:test:ctx-worker-gateway"], undefined);
  assert.equal(scripts["rust:crate:nextest:ctx-worker-gateway"], undefined);
});
