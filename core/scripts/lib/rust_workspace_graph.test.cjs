const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  buildGeneratedPackageScripts,
  buildGeneratedTurboTasks,
  buildWorkspaceGraph,
  collectChangedCrates,
  expandReverseDependencies,
  getCtxHttpSuiteTaskName,
  getTurboTaskNamesForCrates,
} = require("./rust_workspace_graph.cjs");

const coreRoot = path.resolve(__dirname, "../..");

test("workspace graph maps extracted leaf crate paths to crate names", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const crates = collectChangedCrates(graph, [
    "core/crates/ctx-provider-accounts/src/lib.rs",
    "core/crates/ctx-harness-sources/src/lib.rs",
  ]);

  assert.deepEqual(crates, ["ctx-harness-sources", "ctx-provider-accounts"]);
});

test("workspace graph expands reverse dependencies through extracted ctx-http leaves", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const impacted = expandReverseDependencies(graph, ["ctx-provider-accounts"]);

  assert.equal(impacted.includes("ctx-provider-accounts"), true);
  assert.equal(impacted.includes("ctx-harness-sources"), true);
  assert.equal(impacted.includes("ctx-http"), true);
});

test("generated package scripts include per-crate and ctx-http suite tasks", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const scripts = buildGeneratedPackageScripts(graph);

  assert.equal(typeof scripts["rust:crate:clippy:ctx-http"], "string");
  assert.equal(typeof scripts["rust:crate:test:ctx-http"], "string");
  assert.equal(typeof scripts["rust:crate:nextest:ctx-http"], "string");
  assert.equal(typeof scripts["rust:ctx-http:test:workspace-stream"], "string");
});

test("generated turbo tasks include dependency-closure inputs", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const tasks = buildGeneratedTurboTasks(graph);
  const ctxHttpWorkspaceSuiteTask = tasks["rust:ctx-http:test:workspace-stream"];

  assert.equal(Array.isArray(ctxHttpWorkspaceSuiteTask.inputs), true);
  assert.equal(ctxHttpWorkspaceSuiteTask.inputs.includes("crates/ctx-http/src/**"), true);
  assert.equal(
    ctxHttpWorkspaceSuiteTask.inputs.includes("crates/ctx-http/tests/workspace_active_snapshot_http.rs"),
    true,
  );
  assert.equal(ctxHttpWorkspaceSuiteTask.inputs.includes("crates/ctx-provider-accounts/**"), true);
  assert.equal(ctxHttpWorkspaceSuiteTask.inputs.includes("scripts/ctx_http_suite_task.cjs"), true);
});

test("ctx-http test expansion returns explicit suite task names", () => {
  const taskNames = getTurboTaskNamesForCrates(
    ["ctx-http"],
    ["test"],
  );

  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("base")), true);
  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("workspace-stream")), true);
  assert.equal(taskNames.includes(getCtxHttpSuiteTaskName("sandbox-cloud")), true);
});
