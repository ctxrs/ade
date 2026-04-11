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
    "core/crates/ctx-bundled-assets/src/lib.rs",
    "core/crates/ctx-harness-setup/src/lib.rs",
    "core/crates/ctx-provider-accounts/src/lib.rs",
    "core/crates/ctx-provider-auth-import/src/lib.rs",
    "core/crates/ctx-harness-sources/src/lib.rs",
    "core/crates/ctx-runtime-assets/src/lib.rs",
    "core/crates/ctx-sandbox-contract/src/lib.rs",
    "core/crates/ctx-worktree-data-plane/src/lib.rs",
    "core/crates/ctx-workspace-active-snapshot/src/lib.rs",
  ]);

  assert.deepEqual(crates, [
    "ctx-bundled-assets",
    "ctx-harness-setup",
    "ctx-harness-sources",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-runtime-assets",
    "ctx-sandbox-contract",
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

  assert.equal(typeof scripts["rust:crate:clippy:ctx-http"], "string");
  assert.equal(typeof scripts["rust:crate:test:ctx-http"], "string");
  assert.equal(typeof scripts["rust:crate:nextest:ctx-http"], "string");
  assert.equal(typeof scripts["rust:ctx-http:test:workspace-stream"], "string");
});

test("generated turbo tasks include dependency-closure inputs", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const tasks = buildGeneratedTurboTasks(graph);
  const ctxHttpLspSuiteTask = tasks["rust:ctx-http:test:lsp"];
  const ctxHttpProviderAuthSuiteTask = tasks["rust:ctx-http:test:provider-auth"];

  assert.equal(Array.isArray(ctxHttpLspSuiteTask.inputs), true);
  assert.equal(ctxHttpLspSuiteTask.inputs.includes("crates/ctx-http/src/api/lsp/**"), true);
  assert.equal(ctxHttpLspSuiteTask.inputs.includes("crates/ctx-http/src/workspace_runtime/**"), false);
  assert.equal(ctxHttpLspSuiteTask.inputs.includes("crates/ctx-lsp/**"), true);
  assert.equal(ctxHttpLspSuiteTask.inputs.includes("crates/ctx-http/src/api/providers/imports.rs"), false);
  assert.equal(
    ctxHttpLspSuiteTask.inputs.includes("crates/ctx-http/tests/lsp_http_e2e.rs"),
    true,
  );
  assert.equal(ctxHttpLspSuiteTask.inputs.includes("scripts/ctx_http_suite_task.cjs"), true);

  assert.equal(ctxHttpProviderAuthSuiteTask.inputs.includes("crates/ctx-provider-auth-import/**"), true);
  assert.equal(ctxHttpProviderAuthSuiteTask.inputs.includes("crates/ctx-http/src/api/providers/imports.rs"), true);
  assert.equal(
    ctxHttpProviderAuthSuiteTask.inputs.includes("crates/ctx-http/src/workspace_runtime/**"),
    false,
  );

  const ctxHttpWorkspaceStreamSuiteTask = tasks["rust:ctx-http:test:workspace-stream"];
  assert.equal(
    ctxHttpWorkspaceStreamSuiteTask.inputs.includes(
      "crates/ctx-workspace-active-snapshot/**",
    ),
    true,
  );
  assert.equal(
    ctxHttpWorkspaceStreamSuiteTask.inputs.includes(
      "crates/ctx-http/src/workspace_active_snapshot/**",
    ),
    false,
  );
});

test("generated turbo tasks include extracted dependency crates for affected ctx-http suites", () => {
  const graph = buildWorkspaceGraph(coreRoot);
  const tasks = buildGeneratedTurboTasks(graph);

  const providerRuntimeTask = tasks["rust:ctx-http:test:provider-runtime"];
  assert.equal(
    providerRuntimeTask.inputs.includes("crates/ctx-execution-runtime/**"),
    true,
  );
  assert.equal(
    providerRuntimeTask.inputs.includes("crates/ctx-workspace-runtime/**"),
    true,
  );

  const sandboxCloudTask = tasks["rust:ctx-http:test:sandbox-cloud"];
  assert.equal(
    sandboxCloudTask.inputs.includes("crates/ctx-execution-runtime/**"),
    true,
  );
  assert.equal(
    sandboxCloudTask.inputs.includes("crates/ctx-workspace-runtime/**"),
    true,
  );

  const providerAuthTask = tasks["rust:ctx-http:test:provider-auth"];
  assert.equal(
    providerAuthTask.inputs.includes("crates/ctx-provider-install/**"),
    true,
  );
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
