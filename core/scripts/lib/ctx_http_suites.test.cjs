const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  CTX_HTTP_BAZEL_PACKAGE,
  CTX_HTTP_BASE_CHILD_SUITE_NAMES,
  CTX_HTTP_CHECKIN_FANOUT_TARGET_BATCHES_BY_SUITE,
  CTX_HTTP_CHECKIN_FANOUT_TARGETS_BY_SUITE,
  CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS,
  CTX_HTTP_SAFE_CONCURRENT_SUITES,
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITES,
  CTX_HTTP_SUITE_CONCURRENCY_CLASSES,
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  MANUAL_ONLY_CTX_HTTP_TEST_FILES,
  buildCtxHttpSuiteCommands,
  buildCtxHttpSuiteTaskArgs,
  getCtxHttpSuiteCheckinFanoutTargetBatches,
  getCtxHttpSuiteCheckinFanoutTargets,
  getCtxHttpSuiteConcurrencyClass,
  getCtxHttpSuiteExecutionTargets,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteTargets,
  getCtxHttpSuiteTaskName,
  validateCtxHttpSuites,
} = require("./ctx_http_suites.cjs");

const coreRoot = path.resolve(__dirname, "../..");
const canonicalSuiteNames = CTX_HTTP_SUITES.map((suite) => suite.name);

function targetNameFromLabel(target) {
  return String(target).split(":").at(-1);
}

function declaredCheckinFanoutTargetNames(suiteName) {
  if (CTX_HTTP_CHECKIN_FANOUT_TARGETS_BY_SUITE[suiteName]) {
    return CTX_HTTP_CHECKIN_FANOUT_TARGETS_BY_SUITE[suiteName];
  }
  const suite = CTX_HTTP_SUITES.find((candidate) => candidate.name === suiteName);
  if (suite?.type === "integration" && (suite.directTargets || suite.testFiles).length > 0) {
    return suite.directTargets || suite.testFiles;
  }
  return getCtxHttpSuiteTargets(suiteName).map(targetNameFromLabel);
}

test("ctx-http suite assignments cover every integration test exactly once", () => {
  const validation = validateCtxHttpSuites(coreRoot);

  assert.deepEqual(validation, {
    duplicates: [],
    manualOnly: ["attachments_demo_react"],
    missing: [],
    unknown: [],
  });
});

test("ctx-http suite names include the meta all task and stable suite task names", () => {
  const names = getCtxHttpSuiteNames({ includeAll: true });

  assert.deepEqual(names, [...canonicalSuiteNames, "all"]);
  assert.equal(names.includes("provider-runtime-live"), true);
  assert.equal(names.includes("provider-runtime-simulated"), true);
  assert.equal(names.includes("unit-tests-api"), true);
  assert.equal(names.includes("unit-tests-lib"), true);
  assert.equal(names.includes("unit-tests-lib-session-head-large"), true);
  assert.equal(names.includes("bin-tests"), true);
  assert.equal(names.includes("doc-tests"), true);
  assert.equal(names.includes("subagents-local-runtime"), true);
  assert.equal(names.includes("sandbox-runtime-container-e2e"), true);
  assert.equal(names.includes("provider-runtime"), false);
  assert.equal(names.includes("sandbox-cloud"), false);
  assert.throws(() => getCtxHttpSuiteTargets("provider-runtime"), /unknown ctx-http suite/u);
  assert.throws(() => buildCtxHttpSuiteCommands("sandbox-cloud"), /unknown ctx-http suite/u);
  assert.equal(getCtxHttpSuiteTaskName("workspace-stream"), "rust:ctx-http:test:workspace-stream");
});

test("ctx-http suite command builder expands base and meta suites predictably", () => {
  const baseExecutionTargets = getCtxHttpSuiteExecutionTargets("base");
  assert.deepEqual(buildCtxHttpSuiteCommands("base"), [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", ...baseExecutionTargets],
      command: "node",
    },
  ]);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_api`), true);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-api`), false);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_lib_session_head_large`), true);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-lib-session-head-large`), false);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests`), false);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_root_help`), true);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_serve_help`), true);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_init_help`), true);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_self_update_help`), true);
  assert.equal(baseExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:base`), false);

  const allCommands = buildCtxHttpSuiteCommands("all");
  assert.deepEqual(
    allCommands,
    [
      {
        args: ["scripts/run_bazel_pilot.cjs", "test", ...getCtxHttpSuiteExecutionTargets("all")],
        command: "node",
      },
    ],
  );
  assert.equal(new Set(getCtxHttpSuiteTargets("all")).size, getCtxHttpSuiteTargets("all").length);
  assert.equal(new Set(getCtxHttpSuiteExecutionTargets("all")).size, getCtxHttpSuiteExecutionTargets("all").length);
  assert.equal(getCtxHttpSuiteExecutionTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_api`), true);
  assert.equal(getCtxHttpSuiteExecutionTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-api`), false);
  assert.equal(
    getCtxHttpSuiteExecutionTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_lib_session_head_large`),
    true,
  );
  assert.equal(
    getCtxHttpSuiteExecutionTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-lib-session-head-large`),
    false,
  );
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-api`), true);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests`), true);
  assert.equal(getCtxHttpSuiteExecutionTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests`), false);
  assert.equal(getCtxHttpSuiteExecutionTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_root_help`), true);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:doc_tests`), true);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:base`), false);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:attachments-routing`), false);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:workspace-stream`), false);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:repo-vcs`), false);
  assert.deepEqual(CTX_HTTP_BASE_CHILD_SUITE_NAMES, [
    "unit-tests-api",
    "unit-tests-lib",
    "unit-tests-lib-session-head-large",
    "unit-tests-workspace-runtime",
    "unit-tests-daemon-and-scheduler",
    "unit-tests-provider-and-settings",
    "unit-tests-merge-queue",
    "bin-tests",
    "doc-tests",
  ]);
  assert.deepEqual(CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS, [
    `${CTX_HTTP_BAZEL_PACKAGE}:manual-only`,
  ]);
  assert.equal(
    getCtxHttpSuiteTargets("all").some((target) => CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS.includes(target)),
    false,
  );
  assert.equal(
    CTX_HTTP_SUITE_SCRIPT_INPUTS.includes("crates/ctx-http/BUILD.bazel"),
    true,
  );
  assert.equal(
    CTX_HTTP_SUITE_SCRIPT_INPUTS.includes("crates/ctx-http/ctx_http_bazel_tests.bzl"),
    true,
  );
});

test("ctx-http long suites expand to direct Bazel test targets", () => {
  assert.deepEqual(buildCtxHttpSuiteCommands("attachments-routing"), [
    {
      args: [
        "scripts/run_bazel_pilot.cjs",
        "test",
        "//core/crates/ctx-http:global_id_routing_http",
        "//core/crates/ctx-http:image_attachments_http_e2e",
        "//core/crates/ctx-http:workspace_attachments_local_canonical",
      ],
      command: "node",
    },
  ]);
  assert.deepEqual(getCtxHttpSuiteTargets("workspace-stream"), [
    `${CTX_HTTP_BAZEL_PACKAGE}:cache_rehydration`,
    `${CTX_HTTP_BAZEL_PACKAGE}:fault_matrix`,
    `${CTX_HTTP_BAZEL_PACKAGE}:hot_endpoints_no_db`,
    `${CTX_HTTP_BAZEL_PACKAGE}:replay_properties`,
    `${CTX_HTTP_BAZEL_PACKAGE}:task_default_session_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_workspace_stream_subscribe_does_not_reemit_when_worktree_vcs_is_already_computing`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_worktree_vcs_summary_refresh_reloads_live_inventory_before_ready_publish`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_stream_context_window_metrics`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_stream_no_gaps_under_activity`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_stream_stress_active_heads_lag`,
  ]);
  assert.deepEqual(getCtxHttpSuiteTargets("repo-vcs"), [
    `${CTX_HTTP_BAZEL_PACKAGE}:jj_merge_queue_basics`,
    `${CTX_HTTP_BAZEL_PACKAGE}:merge_queue_isolation`,
    `${CTX_HTTP_BAZEL_PACKAGE}:repo_clone_branch_and_safety`,
    `${CTX_HTTP_BAZEL_PACKAGE}:repo_init_initial_commit`,
    `${CTX_HTTP_BAZEL_PACKAGE}:repo_validate_destination`,
    `${CTX_HTTP_BAZEL_PACKAGE}:session_diff_unavailable`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_merge_queue_config_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:worktree_archive_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:worktree_vcs_snapshot`,
  ]);
});

test("ctx-http checkin fanout exposes split unit suite targets without changing suite aliases", () => {
  assert.deepEqual(getCtxHttpSuiteTargets("unit-tests-workspace-runtime"), [
    `${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-workspace-runtime`,
  ]);
  assert.deepEqual(getCtxHttpSuiteCheckinFanoutTargets("unit-tests-workspace-runtime"), [
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_reclaim_idle_runtime_with_parked_containers`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_reuses_running_container`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_prepare_starts_cached_container`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_reclaim_idle_machine`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_reclaim_idle_runtime_with_containers`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_reclaim_ctx_harness_container`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_container_status_avf`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_starts_avf_workspace_vm`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_keeps_avf_workspace_container_ready`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_unknown_machine_state_engine_unreachable`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_running_unreachable_machine_reconfiguration`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_workspace_runtime_recreates_machine_for_memory_profile_change`,
  ]);
  assert.deepEqual(CTX_HTTP_CHECKIN_FANOUT_TARGET_BATCHES_BY_SUITE["bin-tests"], [
    [
      "bin_tests_root_help",
      "bin_tests_serve_help",
      "bin_tests_init_help",
      "bin_tests_self_update_help",
    ],
  ]);
  assert.deepEqual(getCtxHttpSuiteCheckinFanoutTargetBatches("bin-tests"), [
    [
      `${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_root_help`,
      `${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_serve_help`,
      `${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_init_help`,
      `${CTX_HTTP_BAZEL_PACKAGE}:bin_tests_self_update_help`,
    ],
  ]);
  assert.equal(
    getCtxHttpSuiteCheckinFanoutTargetBatches("base").some((batch) =>
      batch.length === 4 && batch.every((target) => target.includes(":bin_tests_"))
    ),
    true,
  );
  assert.deepEqual(getCtxHttpSuiteCheckinFanoutTargetBatches("provider-auth"), [
    [
      `${CTX_HTTP_BAZEL_PACKAGE}:acp_target_scoped_status`,
      `${CTX_HTTP_BAZEL_PACKAGE}:codex_host_import_api`,
      `${CTX_HTTP_BAZEL_PACKAGE}:codex_login_callback_api`,
    ],
    [
      `${CTX_HTTP_BAZEL_PACKAGE}:install_start_contract`,
      `${CTX_HTTP_BAZEL_PACKAGE}:provider_current_ctx_version_regressions`,
      `${CTX_HTTP_BAZEL_PACKAGE}:provider_target_scoped_installs`,
      `${CTX_HTTP_BAZEL_PACKAGE}:subscription_accounts_api`,
    ],
  ]);
  assert.equal(getCtxHttpSuiteCheckinFanoutTargetBatches("base").length, 22);
  assert.equal(getCtxHttpSuiteCheckinFanoutTargets("base").length, 45);
  assert.equal(
    getCtxHttpSuiteCheckinFanoutTargets("base").includes("//core/crates/ctx-managed-installs:unit_tests"),
    true,
  );
  assert.equal(
    getCtxHttpSuiteCheckinFanoutTargets("base").includes("//core/crates/ctx-settings-model:unit_tests"),
    true,
  );
  assert.equal(
    getCtxHttpSuiteCheckinFanoutTargets("base").includes("//core/crates/ctx-settings-service:unit_tests"),
    true,
  );
  assert.equal(
    new Set(getCtxHttpSuiteCheckinFanoutTargets("base")).size,
    getCtxHttpSuiteCheckinFanoutTargets("base").length,
  );
  for (const [suiteName, configuredBatches] of Object.entries(CTX_HTTP_CHECKIN_FANOUT_TARGET_BATCHES_BY_SUITE)) {
    const declaredTargetNames = new Set(declaredCheckinFanoutTargetNames(suiteName));
    for (const configuredBatch of configuredBatches) {
      assert.ok(configuredBatch.length > 1, `${suiteName} should not configure singleton batches`);
      assert.ok(configuredBatch.length <= 4, `${suiteName} batches should stay bounded`);
      for (const targetName of configuredBatch) {
        assert.equal(
          declaredTargetNames.has(targetName),
          true,
          `${suiteName} batch references undeclared target ${targetName}`,
        );
      }
    }
  }
  const workspaceStreamFanout = getCtxHttpSuiteCheckinFanoutTargets("workspace-stream");
  assert.equal(workspaceStreamFanout.includes(`${CTX_HTTP_BAZEL_PACKAGE}:workspace-stream`), false);
  assert.equal(workspaceStreamFanout.includes(`${CTX_HTTP_BAZEL_PACKAGE}:cache_rehydration`), true);
  assert.equal(
    workspaceStreamFanout.includes(
      `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state`,
    ),
    true,
  );
  assert.deepEqual(getCtxHttpSuiteCheckinFanoutTargets("scheduler-runtime"), [
    `${CTX_HTTP_BAZEL_PACKAGE}:assistant_chunk_stream_only`,
    `${CTX_HTTP_BAZEL_PACKAGE}:assistant_message_persistence_faults`,
    `${CTX_HTTP_BAZEL_PACKAGE}:noisy_output_backpressure`,
    `${CTX_HTTP_BAZEL_PACKAGE}:turn_lifecycle_events`,
    `${CTX_HTTP_BAZEL_PACKAGE}:turn_terminal_reconciliation`,
    `${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_scheduler`,
  ]);
  assert.equal(
    getCtxHttpSuiteCheckinFanoutTargets("provider-runtime-simulated").includes(
      `${CTX_HTTP_BAZEL_PACKAGE}:provider_scenarios_offline_crp_fixtures`,
    ),
    true,
  );
});

test("ctx-http suite command builder accepts explicit multi-suite selections", () => {
  const multiExecutionTargets = getCtxHttpSuiteExecutionTargets(["base", "provider-auth"]);
  assert.deepEqual(buildCtxHttpSuiteCommands(["base", "provider-auth"]), [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", ...multiExecutionTargets],
      command: "node",
    },
  ]);
  assert.equal(multiExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:base`), false);
  assert.equal(multiExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit_tests_api`), true);
  assert.equal(multiExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-api`), false);
  assert.equal(multiExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:provider-auth`), false);
  assert.equal(multiExecutionTargets.includes(`${CTX_HTTP_BAZEL_PACKAGE}:codex_login_callback_api`), true);
  assert.deepEqual(buildCtxHttpSuiteTaskArgs(["base", "provider-auth"]), [
    "--suite",
    "base",
    "--suite",
    "provider-auth",
  ]);
  assert.throws(
    () => getCtxHttpSuiteTargets(["all", "base"]),
    /cannot mix 'all' with explicit suites/u,
  );
});

test("ctx-http integration suites declare source ownership and dependency crates", () => {
  assert.equal(CTX_HTTP_SHARED_SOURCE_GLOBS.includes("crates/ctx-http/src/daemon/**"), true);
  assert.deepEqual([...MANUAL_ONLY_CTX_HTTP_TEST_FILES].sort(), [
    "attachments_demo_react",
  ]);

  for (const suite of CTX_HTTP_SUITES) {
    assert.equal(Array.isArray(suite.dependencyCrates), true);
    assert.equal(Array.isArray(suite.sourceGlobs), true);
    if (suite.type === "integration") {
      assert.equal(suite.sourceGlobs.length > 0, true, `${suite.name} should own source globs`);
    }
  }
});

test("ctx-http extracted owner crates route to behavior-owning suites", () => {
  const suiteByName = new Map(CTX_HTTP_SUITES.map((suite) => [suite.name, suite]));

  assert.equal(
    suiteByName.get("unit-tests-api").dependencyCrates.includes("ctx-storage-admission"),
    true,
  );
  assert.equal(
    suiteByName.get("unit-tests-provider-and-settings").dependencyCrates.includes("ctx-managed-installs"),
    true,
  );
  assert.equal(
    suiteByName.get("unit-tests-provider-and-settings").dependencyCrates.includes("ctx-provider-matrix"),
    true,
  );
  assert.equal(
    suiteByName.get("scheduler-runtime").dependencyCrates.includes("ctx-storage-admission"),
    true,
  );
  assert.equal(
    suiteByName.get("scheduler-runtime").dependencyCrates.includes("ctx-mcp-command"),
    true,
  );
  assert.equal(
    suiteByName.get("subagents-control").dependencyCrates.includes("ctx-mcp-command"),
    true,
  );
  assert.equal(
    suiteByName.get("attachments-routing").dependencyCrates.includes("ctx-mcp-command"),
    false,
  );
});

test("ctx-http suite concurrency metadata classifies every concrete suite", () => {
  const validClasses = new Set(Object.values(CTX_HTTP_SUITE_CONCURRENCY_CLASSES));
  for (const suite of CTX_HTTP_SUITES) {
    assert.equal(validClasses.has(getCtxHttpSuiteConcurrencyClass(suite.name)), true, suite.name);
  }
  for (const suiteName of CTX_HTTP_SAFE_CONCURRENT_SUITES) {
    assert.equal(getCtxHttpSuiteConcurrencyClass(suiteName), CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SAFE);
  }
  assert.equal(getCtxHttpSuiteConcurrencyClass("unit-tests-api"), CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SERIALIZED);
  assert.equal(getCtxHttpSuiteConcurrencyClass("base"), CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SERIALIZED);
  assert.throws(() => getCtxHttpSuiteConcurrencyClass("all"), /unknown ctx-http suite/u);
});
