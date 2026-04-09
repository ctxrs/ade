const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  buildGeneratedPackageScripts,
  buildGeneratedTurboTasks,
  buildWorkspaceGraph,
  collectChangedCrates,
  expandReverseDependencies,
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

test("generated package scripts include per-crate clippy, test, and nextest tasks", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const scripts = buildGeneratedPackageScripts(graph);

  assert.equal(typeof scripts["rust:crate:clippy:ctx-http"], "string");
  assert.equal(typeof scripts["rust:crate:test:ctx-http"], "string");
  assert.equal(typeof scripts["rust:crate:nextest:ctx-http"], "string");
});

test("generated turbo tasks include dependency-closure inputs", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const tasks = buildGeneratedTurboTasks(graph);
  const ctxHttpTestTask = tasks["rust:crate:test:ctx-http"];

  assert.equal(Array.isArray(ctxHttpTestTask.inputs), true);
  assert.equal(ctxHttpTestTask.inputs.includes("crates/ctx-http/**"), true);
  assert.equal(ctxHttpTestTask.inputs.includes("crates/ctx-provider-accounts/**"), true);
  assert.equal(ctxHttpTestTask.inputs.includes("scripts/rust_crate_task.cjs"), true);
});

test("turbo task names are stable for impacted crates", () => {
  const taskNames = getTurboTaskNamesForCrates(
    ["ctx-provider-accounts", "ctx-http"],
    ["clippy", "test"],
  );

  assert.deepEqual(taskNames, [
    "rust:crate:clippy:ctx-http",
    "rust:crate:test:ctx-http",
    "rust:crate:clippy:ctx-provider-accounts",
    "rust:crate:test:ctx-provider-accounts",
  ]);
});
