const assert = require("node:assert/strict");
const test = require("node:test");

const {
  DAEMON_EXTRACTION_BLOCKER_PATTERNS,
  API_DOMAIN_RAW_STORE_PATTERNS,
  API_RAW_DAEMON_PATTERNS,
  HANDLE_BACKDOOR_PATTERNS,
  MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS,
  MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  MOBILE_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_TEST_CACHE_ACCESS_PATTERNS,
  SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS,
  TEST_ROUTER_COMPOSITION_PATTERNS,
  TEST_RAW_DAEMON_BUCKET_PATTERNS,
  apiPatternsForPath,
  isTestRustPath,
  mcpDaemonPatternsForPath,
  migratedTestPatternsForPath,
  mobileStorePatternsForPath,
  providerCachePatternsForPath,
  routerCompositionPatternsForPath,
  scanRepo,
  scanRouterComposition,
  scanText,
  sessionFixtureStorePatternsForPath,
  stripCfgTestItems,
} = require("./ctx_http_daemon_boundary_guard.cjs");

test("daemon boundary guard rejects raw daemon state in API code", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn handler(State(state): State<Arc<DaemonState>>) {}
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.equal(violations.length, 4);
  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw DaemonState type",
      "raw DaemonState type",
      "raw daemon state extractor",
      "raw daemon state arc",
    ],
  );
});

test("daemon boundary guard rejects broad daemon handle access in API code", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      async fn handler(State(state): State<DaemonHandle>) {
        let _ = state.core().daemon_handle();
      }
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "broad daemon handle extractor",
      "daemon handle escalation call",
    ],
  );
});

test("daemon boundary guard rejects route-visible store accessors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      async fn handler(State(core): State<CoreHandle>, State(sessions): State<SessionsHandle>) {
        let _ = core.global_store().get_workspace(workspace_id).await;
        let _ = sessions.store_for_session(session_id).await;
        let _ = sessions.route_session_store(session_id).await;
        let _ = workspaces.route_existing_workspace_store(workspace_id).await;
        let _ = sessions.load_session_store(session_id).await;
        let _ = sessions.existing_session_store_for_write(session_id).await;
      }
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "global store accessor",
      "daemon store accessor",
      "daemon store accessor",
      "daemon store accessor",
      "daemon store accessor",
      "daemon store accessor",
    ],
  );
});

test("daemon boundary guard rejects raw Store in daemon-blind API families", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/example.rs",
    contents: `
      use ctx_store::Store;
      async fn handler(store: &Store) {}
    `,
    patterns: API_DOMAIN_RAW_STORE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw ctx_store Store in daemon-blind API family",
      "raw ctx_store Store in daemon-blind API family",
    ],
  );
});

test("daemon boundary guard scopes raw Store ban to migrated API families", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/example.rs").includes(
      API_DOMAIN_RAW_STORE_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/tasks/example.rs").includes(
      API_DOMAIN_RAW_STORE_PATTERNS[0],
    ),
    true,
  );
});

test("daemon boundary guard rejects broad daemon handle fields", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      pub fn router(
        state: impl Into<DaemonHandle>,
      ) {}
      struct ProxyState {
        handle: DaemonHandle,
      }
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["router accepts broad daemon handle", "broad daemon handle field"],
  );
});

test("daemon boundary guard allows cfg(test) helpers to construct daemon state", () => {
  const stripped = stripCfgTestItems(`
    #[cfg(test)]
    fn helper(state: &Arc<DaemonState>) {
      let _ = state;
    }

    fn production_handler(State(state): State<CoreHandle>) {}
  `);

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: stripped,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard ignores test paths", () => {
  assert.equal(isTestRustPath("core/crates/ctx-http/src/api/tasks/lifecycle_tests/archive.rs"), true);
  assert.equal(isTestRustPath("core/crates/ctx-http/src/api/workspaces/tests.rs"), true);
  assert.equal(isTestRustPath("core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs"), true);
  assert.equal(isTestRustPath("core/crates/ctx-http/src/api/workspaces/management.rs"), false);
});

test("daemon boundary guard rejects raw daemon bucket access in test surfaces", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/example.rs",
    contents: `
      async fn helper(state: &DaemonState) {
        let _ = state.core.data_root.clone();
        let _ = state.providers.replace_provider_statuses(Default::default()).await;
      }
    `,
    patterns: TEST_RAW_DAEMON_BUCKET_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw daemon runtime bucket field access",
      "raw daemon runtime bucket field access",
    ],
  );
});

test("daemon boundary guard allows daemon-owned test support accessors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/example.rs",
    contents: `
      async fn helper(state: &DaemonState) {
        let _ = state.test_data_root();
        state.test_upsert_provider_status("fake".into(), status).await;
      }
    `,
    patterns: TEST_RAW_DAEMON_BUCKET_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard rejects raw daemon constructors in migrated test roots", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/auth_boundaries/example.rs",
    contents: `
      async fn helper() {
        let state: Arc<DaemonState> = Arc::new(DaemonState::new(
          root,
          stores,
          map,
          url,
          Some("secret".to_string()),
        ));
        let app = api::router(state.clone());
        let state = common::build_state(root, stores, map, url);
        let app = common::router(state);
        let state = build_state(root, stores, map, url);
        let app = router(state.clone());
        let _ = ctx_daemon::daemon::issue_provider_session_mcp_token(&state, session, workspace, worktree).await;
        let _ = issue_provider_session_mcp_token_with_capabilities(&state, session, workspace, worktree, capabilities).await;
        let _ = revoke_provider_session_mcp_token(token).await;
      }
    `,
    patterns: MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw daemon state constructor in migrated test surface",
      "raw daemon state arc in migrated test surface",
      "raw daemon router wiring in migrated test surface",
      "raw common daemon state helper in migrated test surface",
      "raw common daemon state helper in migrated test surface",
      "raw common router helper in migrated test surface",
      "raw common router helper in migrated test surface",
      "raw provider-session token helper in migrated test surface",
      "raw provider-session token helper in migrated test surface",
      "raw provider-session token helper in migrated test surface",
    ],
  );
});

test("daemon boundary guard rejects alternate raw daemon constructors in migrated test roots", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/provider_routes/example.rs",
    contents: `
      async fn helper() {
        let _ = DaemonState::new_with_public_base_url(root, stores, map, url, public_url, token);
        let _ = DaemonState::new_with_runtime_flags(root, stores, map, url, public_url, token, flags);
      }
    `,
    patterns: MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw daemon state constructor in migrated test surface",
      "raw daemon state constructor in migrated test surface",
    ],
  );
});

test("daemon boundary guard rejects raw scheduler helpers in migrated test roots", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    contents: `
      use ctx_daemon::daemon::scheduler::reconcile_turn_terminal_state;
      use ctx_daemon::daemon::scheduler as sched;
      use ctx_daemon::daemon::{scheduler};
      use ctx_daemon::daemon::{
        merge_queue,
        scheduler as daemon_scheduler,
      };
      use ctx_daemon::daemon as d;
      use ctx_daemon as cd;
      extern crate ctx_daemon as ctxd;

      async fn helper() {
        ctx_daemon::daemon::scheduler::reconcile_turn_failed_on_provider_exit(&state, session, run, turn, "provider_exit").await?;
        daemon::scheduler::reconcile_turn_terminal_state(&state, session, run, turn, "restart").await?;
      }
    `,
    patterns: MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw daemon scheduler helper in migrated test surface",
      "raw daemon scheduler helper in migrated test surface",
      "raw daemon scheduler helper in migrated test surface",
      "raw daemon scheduler helper in migrated test surface",
      "raw daemon scheduler helper in migrated test surface",
      "raw daemon scheduler helper in migrated test surface",
      "raw daemon module alias in migrated test surface",
      "ctx-daemon crate alias in migrated test surface",
      "ctx-daemon crate alias in migrated test surface",
    ],
  );
});

test("daemon boundary guard rejects outer ctx-daemon grouped imports in migrated test roots", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    contents: `
      use ctx_daemon::{daemon as daemon_alias};
      use ctx_daemon::{self as ctxd_grouped};
      use ctx_daemon::{daemon::scheduler};
      use ctx_daemon::{
        daemon::{scheduler as outer_scheduler},
      };
    `,
    patterns: MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw ctx-daemon outer grouped daemon import in migrated test surface",
      "raw ctx-daemon outer grouped daemon import in migrated test surface",
      "raw ctx-daemon outer grouped daemon import in migrated test surface",
      "raw ctx-daemon outer grouped daemon import in migrated test surface",
    ],
  );
});

test("daemon boundary guard allows outer grouped TestDaemon imports in migrated test roots", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    contents: `
      use ctx_daemon::{test_support::TestDaemon};
    `,
    patterns: MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard keeps scheduler grouped-import matching inside braces", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    contents: `
      use ctx_daemon::daemon::{merge_queue};

      async fn helper() {
        let scheduler = "not an import";
        assert_eq!(scheduler, "not an import");
      }
    `,
    patterns: MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard allows TestDaemon provider-session token facade calls", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/auth_boundaries/example.rs",
    contents: `
      async fn helper(state: &TestDaemon) {
        let _ = state.issue_provider_session_mcp_token(session, workspace, worktree).await;
        let _ = state.issue_provider_session_mcp_token_with_capabilities(session, workspace, worktree, capabilities).await;
        let _ = state.revoke_provider_session_mcp_token(token).await;
        state.reconcile_turn_terminal_state_for_test(session, run, turn, "restart").await?;
        state.reconcile_turn_failed_on_provider_exit_for_test(session, run, turn, "provider_exit").await?;
      }
    `,
    patterns: MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard rejects direct API router composition outside test router helpers", () => {
  const violations = scanRouterComposition({
    filePath: "core/crates/ctx-http/tests/fault_matrix.rs",
    contents: `
      fn helper(daemon: &TestDaemon) {
        let app = api::router(daemon.handle());
        let other = ctx_http::api::router(daemon.handle());
        let third = crate::api::router(daemon.handle());
      }
    `,
    patterns: TEST_ROUTER_COMPOSITION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct API router composition outside test router helper",
      "direct API router composition outside test router helper",
      "direct API router composition outside test router helper",
    ],
  );
});

test("daemon boundary guard allows only router_for_daemon in integration common", () => {
  const violations = scanRouterComposition({
    filePath: "core/crates/ctx-http/tests/common/mod.rs",
    contents: `
      pub fn router_for_daemon(daemon: &TestDaemon) -> axum::Router {
        api::router(api::RouteHandles::from_daemon_handle(daemon.handle()))
      }

      pub fn router(state: Arc<DaemonState>) -> axum::Router {
        api::router(state)
      }
    `,
    patterns: TEST_ROUTER_COMPOSITION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["direct API router composition outside test router helper"],
  );
});

test("daemon boundary guard requires router calls to be inside sanctioned helper bodies", () => {
  const violations = scanRouterComposition({
    filePath: "core/crates/ctx-http/tests/common/mod.rs",
    contents: `
      pub fn router_for_daemon(daemon: &TestDaemon) -> axum::Router {
        api::router(api::RouteHandles::from_daemon_handle(daemon.handle()))
      }
      pub fn other_router(daemon: &TestDaemon) -> axum::Router {
        api::router(api::RouteHandles::from_daemon_handle(daemon.handle()))
      }
    `,
    patterns: TEST_ROUTER_COMPOSITION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["direct API router composition outside test router helper"],
  );
});

test("daemon boundary guard allows only sanctioned router helper bodies", () => {
  const cases = [
    {
      filePath: "core/crates/ctx-http/src/lib_tests.rs",
      allowed: `
        fn test_router(daemon: &TestDaemon) -> axum::Router {
          api::router(api::RouteHandles::from_daemon_handle(daemon.handle()))
        }
      `,
      denied: `
        fn other_router(daemon: &TestDaemon) -> axum::Router {
          api::router(api::RouteHandles::from_daemon_handle(daemon.handle()))
        }
      `,
    },
    {
      filePath: "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs",
      allowed: `
        pub(super) fn test_router(state: &TestDaemon) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_handle(state.handle()))
        }
      `,
      denied: `
        pub(super) fn other_router(state: &TestDaemon) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_handle(state.handle()))
        }
      `,
    },
    {
      filePath: "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs",
      allowed: `
        pub(crate) fn spawn_router_for_daemon(listener: tokio::net::TcpListener, daemon: &TestDaemon) {
          let app = ctx_http::api::router(ctx_http::api::RouteHandles::from_daemon_handle(daemon.handle()));
        }
      `,
      denied: `
        pub(crate) fn other_router(handle: DaemonHandle) {
          let app = ctx_http::api::router(ctx_http::api::RouteHandles::from_daemon_handle(handle));
        }
      `,
    },
  ];

  for (const item of cases) {
    const violations = scanRouterComposition({
      filePath: item.filePath,
      contents: `${item.allowed}\n${item.denied}`,
      patterns: TEST_ROUTER_COMPOSITION_PATTERNS,
    });
    assert.deepEqual(
      violations.map((violation) => violation.name),
      ["direct API router composition outside test router helper"],
    );
  }
});

test("daemon boundary guard scopes migrated raw daemon constructor ban", () => {
  assert.equal(
    migratedTestPatternsForPath("core/crates/ctx-http/src/lib_tests/auth_boundaries/example.rs"),
    MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  );
  assert.equal(
    migratedTestPatternsForPath("core/crates/ctx-http/src/lib_tests/provider_routes/example.rs"),
    MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  );
  assert.equal(
    migratedTestPatternsForPath(
      "core/crates/ctx-http/src/lib_tests/health_diagnostics/example.rs",
    ),
    MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  );
  assert.equal(
    migratedTestPatternsForPath("core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs"),
    MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  );
  assert.equal(
    migratedTestPatternsForPath("core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs"),
    MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  );
  for (const filePath of [
    "core/crates/ctx-http/tests/acp_target_scoped_status.rs",
    "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
    "core/crates/ctx-http/tests/assistant_message_persistence_faults.rs",
    "core/crates/ctx-http/src/api/settings.rs",
    "core/crates/ctx-http/src/api/sessions/tests.rs",
    "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
    "core/crates/ctx-http/src/api/providers/login/codex/tests.rs",
    "core/crates/ctx-http/src/api/providers/tests/install_statuses.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/fixtures.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/harness_source.rs",
    "core/crates/ctx-http/src/api/workspaces/tests.rs",
    "core/crates/ctx-http/src/api/tasks.rs",
    "core/crates/ctx-http/src/api/tasks/cleanup_lifecycle_tests.rs",
    "core/crates/ctx-http/src/api/tasks/lifecycle_tests/fixtures.rs",
    "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
    "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs",
    "core/crates/ctx-http/src/lib_tests/cors.rs",
    "core/crates/ctx-http/src/lib_tests/daemon_smoke.rs",
    "core/crates/ctx-http/src/lib_tests/daemon_smoke/streaming/fixture.rs",
    "core/crates/ctx-http/src/lib_tests/execution_launch/example.rs",
    "core/crates/ctx-http/src/lib_tests/log_path_boundaries.rs",
    "core/crates/ctx-http/src/lib_tests/log_path_boundaries/worktree_bootstrap.rs",
    "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    "core/crates/ctx-http/src/lib_tests/org_policy_routes.rs",
    "core/crates/ctx-http/src/lib_tests/run_archive_routes.rs",
    "core/crates/ctx-http/src/lib_tests/session_artifacts.rs",
    "core/crates/ctx-http/src/lib_tests/session_artifacts/download_http/fixture.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http/fixtures.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_large_http.rs",
    "core/crates/ctx-http/src/lib_tests/telemetry_export_boundaries.rs",
    "core/crates/ctx-http/src/lib_tests/update_boundaries.rs",
    "core/crates/ctx-http/src/lib_tests/web_session_routes/fixtures.rs",
    "core/crates/ctx-http/src/lib_tests/workspace_active_routes.rs",
    "core/crates/ctx-http/tests/attachments_demo_react.rs",
    "core/crates/ctx-http/tests/cache_rehydration.rs",
    "core/crates/ctx-http/tests/codex_host_import_api.rs",
    "core/crates/ctx-http/tests/codex_login_callback_api.rs",
    "core/crates/ctx-http/tests/demo_seed_transcript_http.rs",
    "core/crates/ctx-http/tests/common/updates_failure_safety.rs",
    "core/crates/ctx-http/tests/disk_isolated_sandbox_smoke.rs",
    "core/crates/ctx-http/tests/disk_isolated_vcs_integrity.rs",
    "core/crates/ctx-http/tests/fault_matrix.rs",
    "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
    "core/crates/ctx-http/tests/global_id_routing_http.rs",
    "core/crates/ctx-http/tests/harness_container_sandbox_e2e.rs",
    "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
    "core/crates/ctx-http/tests/install_start_contract.rs",
    "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    "core/crates/ctx-http/tests/live_provider_canary.rs",
    "core/crates/ctx-http/tests/memory_leak_e2e.rs",
    "core/crates/ctx-http/tests/merge_queue_isolation.rs",
    "core/crates/ctx-http/tests/message_idempotency.rs",
    "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
    "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
    "core/crates/ctx-http/tests/provider_current_ctx_version_regressions.rs",
    "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
    "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
    "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
    "core/crates/ctx-http/tests/replay_properties.rs",
    "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
    "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
    "core/crates/ctx-http/tests/repo_validate_destination.rs",
    "core/crates/ctx-http/tests/session_diff_unavailable.rs",
    "core/crates/ctx-http/tests/session_model_api.rs",
    "core/crates/ctx-http/tests/subagent_mcp_http.rs",
    "core/crates/ctx-http/tests/subscription_accounts_api.rs",
    "core/crates/ctx-http/tests/system_prompt_append_http.rs",
    "core/crates/ctx-http/tests/task_default_session_http.rs",
    "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
    "core/crates/ctx-http/tests/terminal_ws_reconnect.rs",
    "core/crates/ctx-http/tests/title_generation_local_e2e.rs",
    "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
    "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
    "core/crates/ctx-http/tests/workspace_attachments_local_canonical.rs",
    "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
    "core/crates/ctx-http/tests/workspace_active_snapshot_http.rs",
    "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
    "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
    "core/crates/ctx-http/tests/worktree_archive_http.rs",
  ]) {
    assert.equal(migratedTestPatternsForPath(filePath), MIGRATED_TEST_RAW_DAEMON_PATTERNS);
  }
  assert.deepEqual(
    migratedTestPatternsForPath("core/crates/ctx-http/src/lib_tests/other/example.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct mobile test store access in migrated roots", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    contents: `
      async fn helper(daemon: &TestDaemon) {
        daemon.global_store().get_mobile_access_config().await?;
      }
    `,
    patterns: MOBILE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["direct mobile test global store access"],
  );
});

test("daemon boundary guard scopes mobile store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/lib_tests/auth_boundaries/mobile_tokens.rs",
    "core/crates/ctx-http/src/lib_tests/auth_boundaries/mobile_tokens/registration.rs",
    "core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs",
    "core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs",
    "core/crates/ctx-http/src/lib_tests/mobile_secure_routes.rs",
    "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/pairing/lifecycle.rs",
  ]) {
    assert.deepEqual(mobileStorePatternsForPath(filePath), MOBILE_TEST_STORE_ACCESS_PATTERNS);
  }
  assert.deepEqual(
    mobileStorePatternsForPath("core/crates/ctx-http/src/lib_tests/daemon_smoke/messages.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct provider cache access in migrated roots", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/tests/restarts/harness_source.rs",
    contents: `
      async fn helper(daemon: &TestDaemon) {
        daemon.test_with_provider_options_cache(|cache| cache.clear()).await;
        daemon.test_with_provider_verify_cache(|cache| cache.clear()).await;
        daemon.test_with_provider_usage_cache(|cache| cache.clear()).await;
      }
    `,
    patterns: PROVIDER_TEST_CACHE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct provider options cache closure access",
      "direct provider verify cache closure access",
      "direct provider usage cache closure access",
    ],
  );
});

test("daemon boundary guard scopes provider cache facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/providers/tests/restarts.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/failures.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/harness_source.rs",
    "core/crates/ctx-http/src/lib_tests/provider_routes.rs",
    "core/crates/ctx-http/src/lib_tests/provider_routes/codex_routes.rs",
  ]) {
    assert.deepEqual(providerCachePatternsForPath(filePath), PROVIDER_TEST_CACHE_ACCESS_PATTERNS);
  }
  assert.deepEqual(
    providerCachePatternsForPath("core/crates/ctx-http/src/api/providers/tests/install_statuses.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct MCP daemon test store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http-test-support/src/mcp_daemon/fake.rs",
    contents: `
      async fn helper(daemon: &TestDaemon, stores: &StoreManager, session_id: SessionId) {
        daemon.global_store().list_workspaces().await?;
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(WorkspaceId::new()).await?;
        stores.global().list_workspaces().await?;
        stores.workspace(WorkspaceId::new()).await?;
      }
    `,
    patterns: MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct MCP daemon global store access",
      "direct MCP daemon session store access",
      "direct MCP daemon workspace store access",
      "direct MCP daemon StoreManager global access",
      "direct MCP daemon StoreManager workspace access",
    ],
  );
});

test("daemon boundary guard scopes MCP daemon test facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http-test-support/src/mcp_daemon.rs",
    "core/crates/ctx-http-test-support/src/mcp_daemon/fake.rs",
    "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs",
  ]) {
    assert.deepEqual(mcpDaemonPatternsForPath(filePath), MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS);
  }
  assert.deepEqual(
    mcpDaemonPatternsForPath("core/crates/ctx-http-test-support/src/lib.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct session fixture store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/session_artifacts/root_paths.rs",
    contents: `
      use ctx_store::Store;
      async fn helper(daemon: &TestDaemon, store: &Store) {
        daemon.global_store().list_workspaces().await?;
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.task_session_creation_lock(task_id).await;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
      }
    `,
    patterns: SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct session fixture global store access",
      "direct session fixture session store access",
      "direct session fixture workspace store access",
      "direct session fixture uncached workspace store access",
      "direct session fixture task store access",
      "direct task session creation lock access",
      "direct session fixture StoreManager access",
      "direct session fixture StoreManager global access",
      "direct session fixture StoreManager workspace access",
      "raw session fixture ctx_store Store",
      "raw session fixture ctx_store Store",
    ],
  );
});

test("daemon boundary guard scopes session fixture store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/lib_tests/daemon_smoke.rs",
    "core/crates/ctx-http/src/lib_tests/daemon_smoke/golden_path.rs",
    "core/crates/ctx-http/src/lib_tests/log_path_boundaries.rs",
    "core/crates/ctx-http/src/lib_tests/log_path_boundaries/merge_queue.rs",
    "core/crates/ctx-http/src/lib_tests/session_artifacts.rs",
    "core/crates/ctx-http/src/lib_tests/session_artifacts/download_http/fixture.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http/fixtures.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http/seed/events.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_large_http.rs",
    "core/crates/ctx-http/src/lib_tests/session_head_large_http/seed.rs",
    "core/crates/ctx-http/tests/task_default_session_http.rs",
  ]) {
    assert.deepEqual(
      sessionFixtureStorePatternsForPath(filePath),
      SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    sessionFixtureStorePatternsForPath("core/crates/ctx-http/src/lib_tests/provider_routes.rs"),
    [],
  );
});

test("daemon boundary guard scopes test router composition to sanctioned helpers", () => {
  assert.deepEqual(
    routerCompositionPatternsForPath("core/crates/ctx-http/tests/fault_matrix.rs"),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
  assert.deepEqual(
    routerCompositionPatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
  assert.deepEqual(
    routerCompositionPatternsForPath("core/crates/ctx-http/src/lib_tests.rs"),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
  assert.deepEqual(
    routerCompositionPatternsForPath("core/crates/ctx-http/src/api/router.rs"),
    [],
  );
  assert.deepEqual(
    routerCompositionPatternsForPath(
      "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs",
    ),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
  assert.deepEqual(
    routerCompositionPatternsForPath(
      "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
    ),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
  assert.deepEqual(
    routerCompositionPatternsForPath("core/crates/ctx-http/src/test_support.rs"),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
  assert.deepEqual(
    routerCompositionPatternsForPath("core/crates/ctx-http-test-support/src/mcp_daemon.rs"),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
  assert.deepEqual(
    routerCompositionPatternsForPath(
      "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs",
    ),
    TEST_ROUTER_COMPOSITION_PATTERNS,
  );
});

test("daemon boundary guard rejects DaemonHandle raw-state backdoors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl FromRef<DaemonHandle> for Arc<DaemonState> {}
      impl DaemonHandle {
        fn state(&self) -> &Arc<DaemonState> { todo!() }
      }
      impl CoreHandle {
        fn daemon_handle(&self) -> DaemonHandle { todo!() }
      }
      fn proxy(handle: &DaemonHandle) {
        let router: Arc<axum::Router> = Arc::new(router(handle.clone()));
      }
    `,
    patterns: HANDLE_BACKDOOR_PATTERNS,
  });

  assert.equal(violations.length, 4);
});

test("daemon boundary guard rejects daemon API/router composition ownership", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-daemon/src/daemon/runtime.rs",
    contents: `
      use axum::Router;
      use crate::api;
      fn serve(handle: DaemonHandle) {
        let app = api::router(handle);
        let _ = axum::serve(listener, app);
      }
      impl FromRef<DaemonHandle> for CoreHandle {}
    `,
    patterns: DAEMON_EXTRACTION_BLOCKER_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "daemon imports API module",
      "daemon imports API module",
      "daemon depends on Axum",
      "daemon depends on Axum",
      "daemon owns Axum extractor glue",
    ],
  );
});

test("daemon boundary guard rejects grouped daemon API imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-daemon/src/daemon/runtime.rs",
    contents: `
      use crate::{api, daemon};
      use crate::{
        api as http_api,
        daemon::runtime,
      };
    `,
    patterns: DAEMON_EXTRACTION_BLOCKER_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "daemon imports API module",
      "daemon imports API module",
    ],
  );
});

test("checked-in ctx-http API satisfies the daemon boundary", () => {
  assert.deepEqual(scanRepo(), []);
});
