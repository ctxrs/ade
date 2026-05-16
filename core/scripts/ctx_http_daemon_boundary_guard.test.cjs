const assert = require("node:assert/strict");
const test = require("node:test");

const {
  ACP_CRP_BRIDGE_TOKEN_TEST_STORE_ACCESS_PATTERNS,
  AUTH_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
  CACHE_REHYDRATION_TEST_STORE_ACCESS_PATTERNS,
  DAEMON_EXTRACTION_BLOCKER_PATTERNS,
  API_DOMAIN_RAW_STORE_PATTERNS,
  API_RAW_DAEMON_PATTERNS,
  DEFAULT_SESSION_AND_DIFF_FAKE_DAEMON_FIXTURE_PATTERNS,
  EXECUTION_LAUNCH_TEST_STORE_ACCESS_PATTERNS,
  EXTERNAL_PROVIDER_ROUTE_TEST_STORE_ACCESS_PATTERNS,
  FAKE_DAEMON_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
  FAULT_INJECTION_TEST_STORE_ACCESS_PATTERNS,
  GEMINI_LIVE_MODEL_CATALOG_TEST_STORE_ACCESS_PATTERNS,
  GLOBAL_ID_ROUTING_TEST_STORE_ACCESS_PATTERNS,
  HARNESS_CONTAINER_SANDBOX_TEST_STORE_ACCESS_PATTERNS,
  HANDLE_BACKDOOR_PATTERNS,
  IMAGE_ATTACHMENTS_TEST_STORE_ACCESS_PATTERNS,
  JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS,
  LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS,
  LIVE_PROVIDER_CANARY_TEST_STORE_ACCESS_PATTERNS,
  MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS,
  CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS,
  CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS,
  MERGE_QUEUE_ISOLATION_TEST_STORE_ACCESS_PATTERNS,
  MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS,
  MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
  MOBILE_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS,
  PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS,
  SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS,
  SESSION_VCS_API_ORCHESTRATION_PATTERNS,
  TASK_SESSION_CREATION_API_ADMISSION_PATTERNS,
  WORKSPACE_STREAM_READ_MODEL_API_PATTERNS,
  WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS,
  WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS,
  WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS,
  WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS,
  WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS,
  WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS,
  WORKSPACE_STREAM_LIVE_EVENT_APPLICATION_API_PATTERNS,
  WORKSPACE_STREAM_SUBSCRIPTION_EVENT_API_PATTERNS,
  WORKSPACE_VCS_DEMAND_API_PATTERNS,
  WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS,
  TERMINAL_STREAM_RUNTIME_API_PATTERNS,
  DICTATION_WS_CONFIG_API_PATTERNS,
  WORKSPACE_WS_ADMISSION_API_PATTERNS,
  ORG_POLICY_API_ORCHESTRATION_PATTERNS,
  REPO_ONBOARDING_API_ORCHESTRATION_PATTERNS,
  PROVIDER_AUTH_GLOBAL_ID_FIXTURE_PATTERNS,
  PROVIDERLESS_LIB_ROUTE_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_PROBE_RUNTIME_ENV_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_ROUTE_SETUP_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_SCENARIOS_OFFLINE_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_TARGET_SCOPED_INSTALLS_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_WORKER_REAPING_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_TEST_CACHE_ACCESS_PATTERNS,
  REPLAY_PROPERTIES_TEST_STORE_ACCESS_PATTERNS,
  SCHEDULER_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
  SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS,
  SESSION_MODEL_API_TEST_STORE_ACCESS_PATTERNS,
  SMALL_API_UNIT_TEST_STORE_ACCESS_PATTERNS,
  SMALL_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
  SMALL_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
  SMALL_ROUTE_FIXTURE_PATTERNS,
  STORAGE_ADMISSION_FIXTURE_PATTERNS,
  STREAM_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
  SUBAGENT_MCP_TEST_STORE_ACCESS_PATTERNS,
  SUBSCRIPTION_ACCOUNTS_API_TEST_STORE_ACCESS_PATTERNS,
  TASK_LIFECYCLE_TEST_STORE_ACCESS_PATTERNS,
  TERMINAL_WORKSPACE_STREAM_TEST_STORE_ACCESS_PATTERNS,
  TEST_ROUTER_COMPOSITION_PATTERNS,
  TEST_RAW_DAEMON_BUCKET_PATTERNS,
  UPDATE_ROUTE_FIXTURE_PATTERNS,
  WORKSPACE_MERGE_QUEUE_CONFIG_TEST_STORE_ACCESS_PATTERNS,
  WORKSPACE_RUNTIME_SETTINGS_TEST_STORE_ACCESS_PATTERNS,
  WORKSPACE_ATTACHMENTS_DEMO_TEST_STORE_ACCESS_PATTERNS,
  WORKSPACE_VCS_SETUP_FIXTURE_PATTERNS,
  WORKTREE_ARCHIVE_TEST_STORE_ACCESS_PATTERNS,
  WORKTREE_VCS_SNAPSHOT_TEST_STORE_ACCESS_PATTERNS,
  apiPatternsForPath,
  acpCrpBridgeTokenStorePatternsForPath,
  authBoundaryStorePatternsForPath,
  cacheRehydrationStorePatternsForPath,
  defaultSessionAndDiffFakeDaemonFixturePatternsForPath,
  externalProviderRouteStorePatternsForPath,
  executionLaunchStorePatternsForPath,
  fakeDaemonExternalStorePatternsForPath,
  faultInjectionStorePatternsForPath,
  geminiLiveModelCatalogStorePatternsForPath,
  globalIdRoutingStorePatternsForPath,
  harnessContainerSandboxStorePatternsForPath,
  imageAttachmentsStorePatternsForPath,
  isTestRustPath,
  jjMergeQueueBasicsStorePatternsForPath,
  liveProviderCanaryStorePatternsForPath,
  libTestDataRootFixturePatternsForPath,
  mergeQueueIsolationStorePatternsForPath,
  mcpDaemonPatternsForPath,
  migratedTestPatternsForPath,
  mobileAccessStoreDtoApiPatternsForPath,
  mobileStorePatternsForPath,
  providerAuthGlobalIdFixturePatternsForPath,
  providerScenariosOfflineStorePatternsForPath,
  providerWorkerReapingStorePatternsForPath,
  providerCachePatternsForPath,
  providerProbeRuntimeEnvStorePatternsForPath,
  providerRouteSetupStorePatternsForPath,
  providerTargetScopedInstallsStorePatternsForPath,
  replayPropertiesStorePatternsForPath,
  routerCompositionPatternsForPath,
  scanRepo,
  scanRouterComposition,
  scanText,
  schedulerRuntimeStorePatternsForPath,
  sessionModelApiStorePatternsForPath,
  sessionFixtureStorePatternsForPath,
  smallApiUnitStorePatternsForPath,
  smallExternalStorePatternsForPath,
  smallBoundaryStorePatternsForPath,
  smallRouteFixturePatternsForPath,
  storageAdmissionFixturePatternsForPath,
  streamRuntimeStorePatternsForPath,
  subagentMcpStorePatternsForPath,
  subscriptionAccountsApiStorePatternsForPath,
  taskLifecycleStorePatternsForPath,
  terminalWorkspaceStreamStorePatternsForPath,
  updateRouteFixturePatternsForPath,
  worktreeArchiveStorePatternsForPath,
  workspaceMergeQueueConfigStorePatternsForPath,
  workspaceAttachmentsDemoStorePatternsForPath,
  workspaceRuntimeSettingsStorePatternsForPath,
  workspaceVcsSetupFixturePatternsForPath,
  worktreeVcsSnapshotStorePatternsForPath,
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

test("daemon boundary guard rejects workspace stream read-model orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    contents: `
      async fn handler(state: WorkspaceStreamHandle, workspace_id: WorkspaceId) {
        state.ensure_workspace_active_snapshot_hydrated(workspace_id).await?;
        state.activate_workspace_merge_queue(workspace_id).await;
        let _ = state.workspace_active_snapshot(workspace_id).await;
        let _ = state.workspace_active_heads(workspace_id).await;
        let _ = state.load_workspace_active_snapshot_state(workspace_id).await;
      }
    `,
    patterns: WORKSPACE_STREAM_READ_MODEL_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API owns read-model preparation",
      "workspace stream API owns read-model preparation",
      "workspace stream API owns read-model preparation",
      "workspace stream API owns read-model preparation",
      "workspace stream API owns read-model preparation",
    ],
  );
});

test("daemon boundary guard scopes workspace stream read-model ban to stream transport files", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/replay.rs").includes(
      WORKSPACE_STREAM_READ_MODEL_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    ).includes(WORKSPACE_STREAM_READ_MODEL_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/active.rs").includes(
      WORKSPACE_STREAM_READ_MODEL_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects raw workspace stream subscription-resolution types in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
    contents: `
      use ctx_workspace_active_snapshot::{
        ResolvedWorkspaceActiveSessionReplay,
        ResolvedWorkspaceActiveSessionSubscription,
        ResolvedWorkspaceActiveSubscriptions,
      };
      fn handler(value: ResolvedWorkspaceActiveSessionSubscription) {
        let _ = ResolvedWorkspaceActiveSessionReplay::Reset;
      }
    `,
    patterns: WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API references raw subscription-resolution type",
      "workspace stream API references raw subscription-resolution type",
      "workspace stream API references raw subscription-resolution type",
      "workspace stream API references raw subscription-resolution type",
      "workspace stream API references raw subscription-resolution type",
    ],
  );
});

test("daemon boundary guard scopes workspace stream subscription-plan ban", () => {
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    ).includes(WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/session.rs",
    ).includes(WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream/events.rs").includes(
      WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects workspace stream subscription transaction policy in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
    contents: `
      async fn handler(state: WorkspaceStreamHandle) {
        let resolved = state.resolve_workspace_active_snapshot_subscriptions(workspace_id, message, existing).await?;
        let merged = merge_replayed_and_live_subscriptions(&state, live, replayed);
        let merged = merge_replayed_and_live_subscription_cursors(live, replayed);
        sync_workspace_stream_session_pins(state, current, next).await;
        let attach = next.difference(&current).copied().collect::<Vec<_>>();
        state.attach_session_pin(session_id).await;
        state.detach_session_pin(session_id).await;
      }

      fn merge_replayed_and_live_subscriptions() {}
    `,
    patterns: WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API calls raw subscription resolution directly",
      "workspace stream API merges replayed subscription cursors locally",
      "workspace stream API merges replayed subscription cursors locally",
      "workspace stream API merges replayed subscription cursors locally",
      "workspace stream API defines local replay merge helper",
      "workspace stream API computes subscription pin diffs locally",
      "workspace stream API computes subscription pin set differences locally",
      "workspace stream API mutates session pins directly",
      "workspace stream API mutates session pins directly",
    ],
  );
});

test("daemon boundary guard allows daemon workspace stream subscription transaction DTOs", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    contents: `
      async fn handler(state: WorkspaceStreamHandle) {
        let plan = state
          .plan_workspace_stream_subscription_transaction(workspace_id, message, current, fingerprint)
          .await?;
        let finalization = state.finalize_workspace_stream_subscription_replay(state, live, replayed, sessions);
        state.apply_workspace_stream_session_pin_changes(&pin_changes).await;
        state.release_workspace_stream_session_pins(current).await;
      }
    `,
    patterns: WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes workspace stream subscription transaction ban", () => {
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    ).includes(WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/common/pins.rs").includes(
      WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
    ).includes(WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream/events/route.rs").includes(
      WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects workspace stream replay cursor planning in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
    contents: `
      use ctx_workspace_active_snapshot::replay_cursor_after_live_progress;
      use cursor::{head_only_snapshot_cursor, resume_replay_cursor};

      fn snapshot_cursor(head: WorkspaceActiveSessionHead) {
        let _ = SessionReplayCursor::from_head(head);
      }

      async fn handler(state: WorkspaceStreamHandle) {
        let _ = resume_replay_cursor(after_seq, after_projection_rev);
        let _ = head_only_snapshot_cursor(
          state,
          workspace_id,
          session_id,
          snapshot_cursor,
          include_initial_snapshot,
          live_cursor,
        ).await;
        let _ = replay_cursor_after_live_progress(live_cursor, requested_replay_cursor);
        let _ = state.session_replay_cursor(workspace_id, session_id).await;
        let _ = WorkspaceStreamHandle::session_replay_cursor(&state, workspace_id, session_id).await;
      }
    `,
    patterns: WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API calls active-snapshot replay cursor planner",
      "workspace stream API calls active-snapshot replay cursor planner",
      "workspace stream API owns replay cursor helper",
      "workspace stream API owns replay cursor helper",
      "workspace stream API owns replay cursor helper",
      "workspace stream API builds replay cursor from snapshot head",
      "workspace stream API reads raw session replay cursor",
      "workspace stream API reads raw session replay cursor",
    ],
  );
});

test("daemon boundary guard rejects workspace stream cursor acceptance in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/common/cursor.rs",
    contents: `
      fn accept_session_delta(cursor: &mut SessionCursor, delta: &SessionHeadDelta) -> bool {
        let incoming = SessionReplayCursor::from_delta(delta);
        accept_session_cursor(cursor, incoming)
      }

      fn accept_session_head(cursor: &mut SessionCursor, head: &SessionHeadSnapshot) -> bool {
        accept_session_cursor(cursor, SessionReplayCursor::from_head(head))
      }

      fn accept_session_cursor(cursor: &mut SessionCursor, incoming: SessionReplayCursor) -> bool {
        cursor.last_sent = cursor.last_sent.cover(incoming);
        true
      }
    `,
    patterns: WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API owns cursor acceptance helper",
      "workspace stream API owns cursor acceptance helper",
      "workspace stream API owns cursor acceptance helper",
      "workspace stream API owns cursor acceptance helper",
      "workspace stream API owns cursor acceptance helper",
      "workspace stream API builds replay cursor from snapshot head",
      "workspace stream API builds replay cursor from session delta",
      "workspace stream API merges replay cursors directly",
    ],
  );
});

test("daemon boundary guard scopes workspace stream replay cursor ban", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/common/cursor.rs").includes(
      WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/queue/buffers/head/state.rs").includes(
      WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    ).includes(WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/session.rs",
    ).includes(WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
    ).includes(WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream/events/receiver.rs").includes(
      WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects raw workspace stream replay planning in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
    contents: `
      fn handler(
        state: WorkspaceStreamHandle,
        subscription: WorkspaceStreamResolvedSession,
        resolved_sessions: &[WorkspaceStreamResolvedSession],
      ) {
        let _ = matches!(subscription.intent, WorkspaceActiveSnapshotSessionIntent::Replay);
        let _ = matches!(subscription.replay, WorkspaceStreamSessionReplay::Resume { .. });
        let _ = matches!(subscription.replay, Resume { .. });
        let pending_replay_sessions = resolved_sessions
          .iter()
          .map(|subscription| subscription.session_id)
          .collect::<HashSet<_>>();
        let _ = state.plan_resume_replay_cursor(workspace_id, session_id).await;
        let _ = WorkspaceStreamHandle::head_only_snapshot_cursor(&state, workspace_id, session_id).await;
        let _ = runtime.subscription_state.active_task_sessions.clone();
      }
    `,
    patterns: WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API references raw replay intent policy",
      "workspace stream API references raw replay mode policy",
      "workspace stream API matches raw replay policy variant",
      "workspace stream API matches raw replay policy variant",
      "workspace stream API matches raw replay policy variant",
      "workspace stream API interprets resolved replay subscription fields",
      "workspace stream API interprets resolved replay subscription fields",
      "workspace stream API interprets resolved replay subscription fields",
      "workspace stream API calls replay cursor planner directly",
      "workspace stream API calls replay cursor planner directly",
      "workspace stream API rebuilds pending replay blockers",
      "workspace stream API reads active-task subscription map for replay deferral",
    ],
  );
});

test("daemon boundary guard allows daemon workspace stream replay program DTOs", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
    contents: `
      fn handler(program: WorkspaceStreamReplayProgram, step: WorkspaceStreamReplayStep) {
        let _ = state.plan_workspace_stream_replay_program(workspace_id).await;
        match step {
          WorkspaceStreamReplayStep::HeadOnly { session_id, cursor } => {}
          WorkspaceStreamReplayStep::Replay { session_id, replay_cursor, .. } => {}
          WorkspaceStreamReplayStep::NoReplayRequired { session_id } => {}
        }
        let _ = program.pending_replay_sessions;
      }
    `,
    patterns: WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes workspace stream replay-program planning ban", () => {
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    ).includes(WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
    ).includes(WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/session.rs",
    ).includes(WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream/events.rs").includes(
      WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects workspace stream event-routing predicates in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/events/route.rs",
    contents: `
      use ctx_workspace_active_snapshot::{
        primary_session_id_for_active_task,
        workspace_stream_event_blocks_pending_replay,
      };
      use ctx_daemon::daemon::workspaces::stream::{
        should_stream_head_delta as route_head_delta,
      };

      fn event_snapshot_rev(event: &WorkspaceActiveSnapshotEvent) -> Option<i64> {
        Some(1)
      }

      fn handler(event: WorkspaceActiveSnapshotEvent) {
        let _ = primary_session_id_for_active_task(task);
        let _ = workspace_stream_event_blocks_pending_replay(event, pending, active);
        let _ = should_stream_head_delta(active, explicit, foreground, session_id);
        let _ = filter_partial_delta_for_active_tasks(delta, active, foreground);
        let _ = is_priority_control_event(event, foreground);
        let _ = is_foreground_session(foreground, session_id);
        let _ = event_snapshot_rev(event);
      }
    `,
    patterns: WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API imports active-snapshot event predicate helper",
      "workspace stream API imports active-snapshot event predicate helper",
      "workspace stream API owns event-routing predicate",
      "workspace stream API owns event-routing predicate",
      "workspace stream API owns event-routing predicate",
      "workspace stream API owns event-routing predicate",
      "workspace stream API owns event-routing predicate",
      "workspace stream API owns event-routing predicate",
      "workspace stream API owns event-routing predicate",
      "workspace stream API owns event-routing predicate",
      "workspace stream API defines event-routing predicate",
    ],
  );
});

test("daemon boundary guard scopes workspace stream event-routing predicate ban", () => {
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/events/route.rs",
    ).includes(WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/queue/partials.rs").includes(
      WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/common/rev.rs").includes(
      WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/queue/partials/coalesce.rs",
    ).includes(WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS[0]),
    false,
  );
});

test("daemon boundary guard rejects direct workspace stream event routing in HTTP route files", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
    contents: `
      use WorkspaceActiveSnapshotEvent::*;
      fn route_head_delta() {}
      fn route_summary_delta() {}
      fn route_control_event() {}

      async fn handler(state: WorkspaceStreamHandle, event: WorkspaceActiveSnapshotEvent) {
        let _ = state.plan_workspace_stream_event_route(&subscription_state, event);
        let _ = state.accept_session_delta_cursor(cursor, delta);
        let _ = state.accept_session_head_cursor(cursor, head);
        let _ = state.apply_workspace_stream_subscription_event(workspace_id, state, cursors, event).await;
        match event {
          WorkspaceActiveSnapshotEvent::SessionHeadDelta { .. } => {}
          WorkspaceActiveSnapshotEvent::SessionSummaryDelta { .. } => {}
          WorkspaceActiveSnapshotEvent::SessionGap { .. } => {}
          WorkspaceActiveSnapshotEvent::SessionHeadSeed { .. } => {}
          WorkspaceActiveSnapshotEvent::SessionRemoved { .. } => {}
          SessionHeadDelta { .. } => {}
          SessionSummaryDelta { .. } => {}
          SessionGap { .. } => {}
          SessionHeadSeed { .. } => {}
          SessionRemoved { .. } => {}
          _ => {}
        }
      }
    `,
    patterns: [
      ...WORKSPACE_STREAM_LIVE_EVENT_APPLICATION_API_PATTERNS,
      ...WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS,
    ],
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API calls live event route planning directly",
      "workspace stream API calls live event cursor acceptance directly",
      "workspace stream API calls live event cursor acceptance directly",
      "workspace stream API calls subscription event application directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API matches event-routing domain event directly",
      "workspace stream API owns event route helper",
      "workspace stream API owns event route helper",
      "workspace stream API owns event route helper",
    ],
  );
});

test("daemon boundary guard scopes direct workspace stream event routing ban", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream/events.rs").includes(
      WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/events/route.rs",
    ).includes(WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/session.rs",
    ).includes(WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/events/route/tests.rs",
    ).includes(WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS[0]),
    false,
  );
});

test("daemon boundary guard scopes live event application ban", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream/events.rs").includes(
      WORKSPACE_STREAM_LIVE_EVENT_APPLICATION_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/session.rs",
    ).includes(WORKSPACE_STREAM_LIVE_EVENT_APPLICATION_API_PATTERNS[0]),
    false,
  );
});

test("daemon boundary guard allows daemon live event application method", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
    contents: `
      fn handler(state: WorkspaceStreamHandle, event: WorkspaceActiveSnapshotEvent) {
        let _ = state.event_snapshot_rev(&event);
        let _ = state.apply_workspace_stream_live_event(workspace_id, subscription_state, cursors, event);
      }
    `,
    patterns: [
      ...WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS,
      ...WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS,
    ],
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard rejects workspace stream subscription event mutation in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
    contents: `
      fn update(runtime: &mut WorkspaceStreamRuntime, event: WorkspaceActiveSnapshotEvent) {
        runtime.subscription_state.active_task_sessions.insert(task_id, session_id);
        runtime.subscription_state.explicit_sessions.remove(&session_id);
        runtime.subscription_state.foreground_session_ids = None;
        runtime.subscription_state.replay_sessions.clear();
        match event {
          WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { .. } => {}
          WorkspaceActiveSnapshotEvent::ActiveTaskDelete { .. } => {}
          WorkspaceActiveSnapshotEvent::TaskDelta { .. } => {}
          _ => {}
        }
      }

      fn remove_active_task_subscription_if_unused() {}
      fn remove_runtime_subscription() {}
    `,
    patterns: WORKSPACE_STREAM_SUBSCRIPTION_EVENT_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace stream API mutates subscription state domain set",
      "workspace stream API mutates subscription state domain set",
      "workspace stream API mutates subscription state domain set",
      "workspace stream API mutates subscription state domain set",
      "workspace stream API matches active subscription event domain",
      "workspace stream API matches active subscription event domain",
      "workspace stream API matches active subscription event domain",
      "workspace stream API owns active subscription mutation helper",
      "workspace stream API owns active subscription mutation helper",
    ],
  );
});

test("daemon boundary guard scopes workspace stream subscription event mutation ban", () => {
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
    ).includes(WORKSPACE_STREAM_SUBSCRIPTION_EVENT_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream/events/route.rs").includes(
      WORKSPACE_STREAM_SUBSCRIPTION_EVENT_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects workspace VCS demand planning in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/client.rs",
    contents: `
      impl WorkspaceVcsRuntime {
        fn active_worktree_ids(&self) {}
      }

      async fn handler(state: WorkspacesHandle, previous: HashSet<WorktreeId>, next: HashSet<WorktreeId>) {
        let filtered = state.filter_workspace_worktree_ids(workspace_id, ids).await;
        state.update_worktree_vcs_activity(&previous, &next).await;
        state.update_worktree_vcs_open_panes(&previous, &next).await;
        let added = next.difference(&previous).copied().collect::<Vec<_>>();
      }
    `,
    patterns: WORKSPACE_VCS_DEMAND_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace VCS API calls raw demand filter directly",
      "workspace VCS API mutates demand refs directly",
      "workspace VCS API mutates demand refs directly",
      "workspace VCS API computes demand set differences locally",
      "workspace VCS API owns active demand helper",
    ],
  );
});

test("daemon boundary guard scopes workspace VCS demand planning ban", () => {
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/client.rs",
    ).includes(WORKSPACE_VCS_DEMAND_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/runtime.rs",
    ).includes(WORKSPACE_VCS_DEMAND_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_vcs/socket.rs").includes(
      WORKSPACE_VCS_DEMAND_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects workspace VCS live routing in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs/socket.rs",
    contents: `
      async fn handler(runtime: WorkspaceVcsRuntime, snapshot: WorktreeVcsSnapshot) {
        if runtime.detail_worktree_ids.contains(&snapshot.worktree_id) {}
        if runtime.summary_worktree_ids.contains(&snapshot.worktree_id) {}
      }

      fn seeds(summary_worktree_ids: HashSet<WorktreeId>, detail_worktree_ids: HashSet<WorktreeId>) {
        let _ = summary_worktree_ids.union(&detail_worktree_ids).collect::<Vec<_>>();
        let _ = detail_worktree_ids.contains(&WorktreeId::new());
      }
    `,
    patterns: WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace VCS API routes snapshots from raw demand sets",
      "workspace VCS API routes snapshots from raw demand sets",
      "workspace VCS API plans snapshot seeds from raw demand sets",
      "workspace VCS API plans snapshot seeds from raw demand sets",
    ],
  );
});

test("daemon boundary guard scopes workspace VCS live routing ban", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_vcs/socket.rs").includes(
      WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/snapshots.rs",
    ).includes(WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/client.rs",
    ).includes(WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS[0]),
    false,
  );
});

test("daemon boundary guard rejects raw terminal stream runtime access in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/terminal/socket.rs",
    contents: `
      use ctx_transport_runtime::terminals::{TerminalSessionHandle, TerminalStatusEvent};
      use tokio::sync::broadcast;
      use tokio::sync::broadcast::Receiver;

      async fn handler(
        session: TerminalSessionHandle,
        mut output_rx: broadcast::Receiver<Vec<u8>>,
        mut status_rx: broadcast::Receiver<TerminalStatusEvent>,
        mut imported_rx: Receiver<Vec<u8>>,
      ) {
        session.mark_client_connected();
        session.send_input(vec![]);
        session.resize(80, 24);
        let _ = session.output_receiver();
        let _ = session.status_receiver();
        let _ = session.snapshot();
        let _ = session.output_snapshot();
        let _ = session.output_snapshot_tail(4096);
        session.mark_client_disconnected();
      }
    `,
    patterns: TERMINAL_STREAM_RUNTIME_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "terminal WS API imports raw terminal session handle",
      "terminal WS API imports raw terminal session handle",
      "terminal WS API imports raw terminal status event",
      "terminal WS API imports raw terminal status event",
      "terminal WS API uses raw terminal broadcast receiver",
      "terminal WS API uses raw terminal broadcast receiver",
      "terminal WS API uses raw terminal broadcast receiver",
      "terminal WS API calls raw terminal lifecycle or command methods",
      "terminal WS API calls raw terminal lifecycle or command methods",
      "terminal WS API calls raw terminal lifecycle or command methods",
      "terminal WS API calls raw terminal lifecycle or command methods",
      "terminal WS API opens raw terminal receivers",
      "terminal WS API opens raw terminal receivers",
      "terminal WS API reads raw terminal snapshots",
      "terminal WS API reads raw terminal snapshots",
      "terminal WS API reads raw terminal snapshots",
    ],
  );
});

test("daemon boundary guard scopes terminal stream runtime ban to terminal websocket API", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/ws/terminal.rs",
    "core/crates/ctx-http/src/api/ws/terminal/socket.rs",
    "core/crates/ctx-http/src/api/ws/terminal/socket/output.rs",
    "core/crates/ctx-http/src/api/ws/tests/ws_queue_tests.rs",
  ]) {
    assert.equal(
      apiPatternsForPath(filePath).includes(TERMINAL_STREAM_RUNTIME_API_PATTERNS[0]),
      true,
    );
  }

  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_stream.rs").includes(
      TERMINAL_STREAM_RUNTIME_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects dictation websocket config policy in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/dictation_livekit/settings.rs",
    contents: `
      use ctx_settings_model::DictationProvider;
      use ctx_transport_runtime::dictation_livekit::{
        normalize_livekit_dictation_config, LiveKitDictationConfigInput,
      };

      async fn handler(state: CoreHandle) {
        let settings = state.load_settings().await?;
        let settings = load_settings(&store).await?;
        let settings = settings::load_settings(&store).await?;
        let provider = DictationProvider::LiveKitInference;
        let input = LiveKitDictationConfigInput {
          api_key: String::new(),
          api_secret: None,
          base_url: String::new(),
          model: String::new(),
          language: String::new(),
        };
        let _ = normalize_livekit_dictation_config(input);
      }
    `,
    patterns: DICTATION_WS_CONFIG_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "dictation WS API loads settings directly",
      "dictation WS API loads settings directly",
      "dictation WS API loads settings directly",
      "dictation WS API interprets dictation provider policy",
      "dictation WS API interprets dictation provider policy",
      "dictation WS API normalizes LiveKit config directly",
      "dictation WS API imports LiveKit config input",
      "dictation WS API imports LiveKit config input",
    ],
  );
});

test("daemon boundary guard scopes dictation websocket config policy ban", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/ws/dictation_livekit.rs",
    "core/crates/ctx-http/src/api/ws/dictation_livekit/settings.rs",
    "core/crates/ctx-http/src/api/ws/dictation_livekit/bridge/livekit.rs",
  ]) {
    assert.equal(
      apiPatternsForPath(filePath).includes(DICTATION_WS_CONFIG_API_PATTERNS[0]),
      true,
    );
  }

  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/settings.rs").includes(
      DICTATION_WS_CONFIG_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects workspace websocket admission policy in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_active.rs",
    contents: `
      async fn require_workspace_active_stream_access(
        state: &WorkspaceStreamHandle,
        workspace_id: WorkspaceId,
      ) -> Result<(), StatusCode> {
        let exists = state.workspace_exists(workspace_id).await?;
        let exists = WorkspaceStreamHandle::workspace_exists(state, workspace_id).await?;
        let exists = WorkspacesHandle::workspace_exists(workspaces, workspace_id).await?;
        Ok(())
      }
    `,
    patterns: WORKSPACE_WS_ADMISSION_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace WS API performs direct workspace existence admission",
      "workspace WS API performs direct workspace existence admission",
      "workspace WS API performs direct workspace existence admission",
      "workspace WS API defines local stream access helper",
    ],
  );
});

test("daemon boundary guard scopes workspace websocket admission ban", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/ws/workspace_active.rs",
    "core/crates/ctx-http/src/api/ws/workspace_vcs.rs",
  ]) {
    assert.equal(
      apiPatternsForPath(filePath).includes(WORKSPACE_WS_ADMISSION_API_PATTERNS[0]),
      true,
    );
  }

  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/ws/workspace_vcs/socket.rs").includes(
      WORKSPACE_WS_ADMISSION_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects org policy orchestration in HTTP routes", () => {
  const snapshotViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/org_policy/snapshots.rs",
    contents: `
      async fn handler(state: CoreHandle, mut enrollment: DaemonEnrollment) {
        let enrollment = state.get_daemon_enrollment_by_org_id(org_id).await?;
        let enrollment = CoreHandle::get_daemon_enrollment_by_org_id(&state, org_id).await?;
        ctx_org_policy::signature::verify_policy_snapshot_signature(&enrollment, &snapshot)?;
        let stored = state.upsert_org_policy_snapshot(snapshot).await?;
        let stored = CoreHandle::upsert_org_policy_snapshot(&state, snapshot).await?;
        enrollment.active_policy_snapshot_id = Some(stored.id);
        enrollment.updated_at = chrono::Utc::now();
        enrollment.updated_at = Utc::now();
        state.upsert_daemon_enrollment(enrollment).await?;
        CoreHandle::upsert_daemon_enrollment(&state, enrollment).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/snapshots.rs"),
  });

  assert.deepEqual(
    snapshotViolations.map((violation) => violation.name),
    [
      "org policy API verifies policy snapshot signatures directly",
      "org policy API loads daemon enrollment directly for snapshot or overlay orchestration",
      "org policy API loads daemon enrollment directly for snapshot or overlay orchestration",
      "org policy snapshot API stores snapshots directly",
      "org policy snapshot API stores snapshots directly",
      "org policy snapshot API mutates daemon enrollment directly",
      "org policy snapshot API mutates daemon enrollment directly",
      "org policy snapshot API mutates active snapshot id directly",
      "org policy snapshot API refreshes enrollment timestamp directly",
      "org policy snapshot API refreshes enrollment timestamp directly",
    ],
  );

  const overlayViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/org_policy/workspace_overlay.rs",
    contents: `
      async fn handler(core: CoreHandle, workspaces: WorkspacesHandle) {
        let enrollment = core.get_daemon_enrollment_by_org_id(org_id).await?;
        let enrollment = CoreHandle::get_daemon_enrollment_by_org_id(&core, org_id).await?;
        let overlay = workspaces.upsert_workspace_policy_overlay(overlay).await?;
        let overlay = WorkspacesHandle::upsert_workspace_policy_overlay(&workspaces, overlay).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/workspace_overlay.rs"),
  });

  assert.deepEqual(
    overlayViolations.map((violation) => violation.name),
    [
      "org policy API loads daemon enrollment directly for snapshot or overlay orchestration",
      "org policy API loads daemon enrollment directly for snapshot or overlay orchestration",
      "org policy workspace overlay API writes overlays without daemon admission",
      "org policy workspace overlay API writes overlays without daemon admission",
    ],
  );

  const enrollmentViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/org_policy/enrollments.rs",
    contents: `
      async fn handler(state: CoreHandle, mut enrollment: DaemonEnrollment) {
        if !matches!(enrollment.plan_type, PlanType::Team | PlanType::Enterprise) {
          return Err(());
        }
        if matches!(enrollment.plan_type, PlanType::Enterprise | PlanType::Team) {
          return Ok(());
        }
        if enrollment.plan_type == PlanType::Team || enrollment.plan_type == PlanType::Enterprise {
          return Ok(());
        }
        if enrollment.plan_type == PlanType::Team
          || enrollment.plan_type == PlanType::Enterprise
        {
          return Ok(());
        }
        if enrollment.policy_signing_key.trim().is_empty() {
          return Err(());
        }
        let missing_key = enrollment.policy_signing_key.trim().is_empty();
        enrollment.updated_at = chrono::Utc::now();
        let now = Utc::now();
        enrollment.updated_at = now;
        state.upsert_daemon_enrollment(enrollment).await?;
        CoreHandle::upsert_daemon_enrollment(&state, enrollment).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/enrollments.rs"),
  });

  assert.deepEqual(
    enrollmentViolations.map((violation) => violation.name),
    [
      "org policy enrollment API validates plan eligibility directly",
      "org policy enrollment API validates signing key directly",
      "org policy enrollment API validates signing key directly",
      "org policy enrollment API refreshes enrollment timestamp directly",
      "org policy enrollment API refreshes enrollment timestamp directly",
      "org policy enrollment API persists enrollment without checked daemon validation",
      "org policy enrollment API persists enrollment without checked daemon validation",
    ],
  );

  const planValidationVariants = [
    `
      async fn handler(enrollment: DaemonEnrollment) {
        if matches!(enrollment.plan_type, PlanType::Enterprise | PlanType::Team) {
          return Ok(());
        }
      }
    `,
    `
      async fn handler(enrollment: DaemonEnrollment) {
        if enrollment.plan_type == PlanType::Team || enrollment.plan_type == PlanType::Enterprise {
          return Ok(());
        }
      }
    `,
    `
      async fn handler(enrollment: DaemonEnrollment) {
        if enrollment.plan_type == PlanType::Team
          || enrollment.plan_type == PlanType::Enterprise
        {
          return Ok(());
        }
      }
    `,
    `
      async fn handler(enrollment: DaemonEnrollment) {
        if matches!(enrollment.plan_type, PlanType::FreeLocal | PlanType::Pro) {
          return Err(());
        }
      }
    `,
    `
      async fn handler(enrollment: DaemonEnrollment) {
        if enrollment.plan_type == PlanType::Pro {
          return Err(());
        }
      }
    `,
  ];
  for (const contents of planValidationVariants) {
    const violations = scanText({
      filePath: "core/crates/ctx-http/src/api/org_policy/enrollments.rs",
      contents,
      patterns: apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/enrollments.rs"),
    });
    assert(
      violations
        .map((violation) => violation.name)
        .includes("org policy enrollment API validates plan eligibility directly"),
    );
  }
});

test("daemon boundary guard scopes org policy orchestration bans", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/snapshots.rs").includes(
      ORG_POLICY_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/org_policy.rs").includes(
      ORG_POLICY_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/workspace_overlay.rs").includes(
      ORG_POLICY_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/enrollments.rs").includes(
      ORG_POLICY_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );

  const enrollmentViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/org_policy/enrollments.rs",
    contents: `
      impl From<DaemonEnrollment> for DaemonEnrollmentResponse {
        fn from(enrollment: DaemonEnrollment) -> Self {
          Self {
            policy_signing_key_present: !enrollment.policy_signing_key.trim().is_empty(),
          }
        }
      }
      async fn handler(state: CoreHandle, enrollment: DaemonEnrollment) {
        state.upsert_daemon_enrollment_checked(enrollment).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/enrollments.rs"),
  });
  assert.deepEqual(enrollmentViolations, []);

  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/settings.rs").includes(
      ORG_POLICY_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects repo onboarding orchestration in HTTP routes", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/repo/init.rs",
    contents: `
      use ctx_workspace_services::repo_onboarding::{
        RepoInitRequest,
        RepoCloneRequest,
        RepoValidateDestinationRequest,
      };
      use ctx_workspace_services as cws;
      async fn handler(state: CoreHandle, error: RepoGitCommandError, path_error: RepoOnboardingPathError) {
        let path = ctx_workspace_services::repo_onboarding::initialize_repo(RepoInitRequest {
          path: &req.path,
          allow_existing: false,
          allow_non_empty: false,
        }).await?;
        let path = repo_onboarding::clone_repo(RepoCloneRequest {
          repo_url: &req.repo_url,
          dest_parent: &req.dest_parent,
          branch: None,
          dest_name: None,
        }).await?;
        let path = service::validate_repo_destination(RepoValidateDestinationRequest {
          path: &req.path,
          must_not_exist: false,
          require_empty_if_exists: false,
        }).await?;
        let status = cws::repo_onboarding::inspect_repo_status(&req.path).await?;
        let staging = ctx_workspace_services::repo_onboarding::create_repo_staging_path(state.data_root()).await?;
        let error = logs::redact_sensitive(&error.failed_message().unwrap());
        let message = path_error.message().to_string();
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/repo/init.rs"),
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "repo onboarding API calls workspace-service onboarding directly",
      "repo onboarding API calls workspace-service onboarding directly",
      "repo onboarding API calls workspace-service onboarding directly",
      "repo onboarding API calls workspace-service onboarding directly",
      "repo onboarding API calls workspace-service onboarding directly",
      "repo onboarding API calls workspace-service onboarding directly",
      "repo onboarding API uses workspace-service onboarding DTOs directly",
      "repo onboarding API uses workspace-service onboarding DTOs directly",
      "repo onboarding API uses workspace-service onboarding DTOs directly",
      "repo onboarding API uses workspace-service onboarding DTOs directly",
      "repo onboarding API uses workspace-service onboarding DTOs directly",
      "repo onboarding API uses workspace-service onboarding DTOs directly",
      "repo onboarding API reads daemon data root directly",
      "repo onboarding API redacts workflow errors directly",
      "repo onboarding API inspects workspace-service git errors directly",
      "repo onboarding API inspects workspace-service path errors directly",
    ],
  );
});

test("daemon boundary guard scopes repo onboarding orchestration bans", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/repo/init.rs").includes(
      REPO_ONBOARDING_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/repo.rs").includes(
      REPO_ONBOARDING_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/repo/init.rs",
    contents: `
      use ctx_daemon::daemon::repo_onboarding::DaemonRepoInitRequest;
      async fn handler(workspaces: WorkspacesHandle, error: RepoOnboardingError) {
        let path = workspaces.initialize_repo(DaemonRepoInitRequest {
          path: req.path,
          allow_existing: false,
          allow_non_empty: false,
        }).await?;
        let status = match error.kind() {
          RepoOnboardingErrorKind::BadRequest => StatusCode::BAD_REQUEST,
          RepoOnboardingErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ApiErrorResp { error: error.message().to_string() };
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/repo/init.rs"),
  });
  assert.deepEqual(violations, []);

  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/settings.rs").includes(
      REPO_ONBOARDING_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
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
      filePath: "core/crates/ctx-http/src/api/workspaces/tests.rs",
      allowed: `
        fn test_router(daemon: &TestDaemon) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_handle(
            daemon.handle(),
          ))
        }
      `,
      denied: `
        fn other_router(daemon: &TestDaemon) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_handle(daemon.handle()))
        }
      `,
    },
    {
      filePath: "core/crates/ctx-http/src/test_support.rs",
      allowed: `
        pub(crate) fn router(&self) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_handle(
            self.daemon.handle(),
          ))
        }
      `,
      denied: `
        pub(crate) fn other_router(&self) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_handle(self.daemon.handle()))
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
        let stores = StoreManager::open(data_dir.path()).await?;
        daemon.global_store().get_mobile_access_config().await?;
        test_daemon(data_dir, stores, None);
        test_daemon_for_test(data_dir, None).await;
      }
    `,
    patterns: MOBILE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct mobile test global store access",
      "legacy mobile test daemon construction",
      "raw mobile test StoreManager",
    ],
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

test("daemon boundary guard rejects provider-route direct store setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/provider_routes/codex_routes.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn helper(daemon: TestDaemon, stores: StoreManager) {
        let stores = StoreManager::open(data_dir.path()).await?;
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.store_for_worktree(worktree_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace_uncached(workspace_id).await?;
        daemon.handle().providers();
        let handle = daemon.handle();
        handle.sessions();
        handle.workspaces();
        handle.tasks();
        let daemon = TestDaemon::new_with_providers_for_test(data_root, HashMap::new(), "http://127.0.0.1:0".to_string(), None).await?;
        let daemon = test_daemon_for_test(data_root, None).await;
        let daemon = test_daemon_with_fake_provider_for_test(data_root, None).await;
        let app = test_router(&daemon);
        let app = crate::api::router(RouteHandles::from_daemon_handle(daemon.handle()));
        let app = fixture.router();
        let _raw: Store;
      }
    `,
    patterns: PROVIDER_ROUTE_SETUP_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct provider-route global store access",
      "direct provider-route session store access",
      "direct provider-route workspace store access",
      "direct provider-route uncached workspace store access",
      "direct provider-route task store access",
      "direct provider-route worktree store access",
      "direct provider-route StoreManager access",
      "direct provider-route StoreManager access",
      "direct provider-route StoreManager access",
      "direct provider-route StoreManager access",
      "direct provider-route StoreManager global access",
      "direct provider-route StoreManager global access",
      "direct provider-route StoreManager workspace access",
      "direct provider-route StoreManager workspace access",
      "direct provider-route handle access",
      "direct provider-route handle access",
      "direct provider-route handle access",
      "direct provider-route handle access",
      "direct provider-route handle access",
      "direct provider-route TestDaemon construction",
      "direct provider-route lib-test daemon helper",
      "direct provider-route lib-test daemon helper",
      "direct provider-route router composition",
      "direct provider-route router composition",
      "raw provider-route ctx_store Store",
      "raw provider-route ctx_store Store",
    ],
  );
});

test("daemon boundary guard scopes provider-route setup roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/lib_tests/provider_routes.rs",
    "core/crates/ctx-http/src/lib_tests/provider_routes/codex_routes.rs",
    "core/crates/ctx-http/src/lib_tests/provider_routes/agent_server_config/status_auth.rs",
  ]) {
    assert.deepEqual(
      providerRouteSetupStorePatternsForPath(filePath),
      PROVIDER_ROUTE_SETUP_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    providerRouteSetupStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/auth_boundaries/daemon_http.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects auth-boundary direct store setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/auth_boundaries/daemon_http.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn helper(daemon: TestDaemon, stores: StoreManager) {
        let stores = StoreManager::open(data_dir.path()).await?;
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.store_for_worktree(worktree_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace_uncached(workspace_id).await?;
        daemon.handle().sessions();
        let handle = daemon.handle();
        handle.providers();
        handle.workspaces();
        handle.tasks();
        let daemon = TestDaemon::new_with_providers_for_test(data_root, HashMap::new(), "http://127.0.0.1:0".to_string(), None).await?;
        let daemon = test_daemon_for_test(data_root, None).await;
        let daemon = test_daemon_with_fake_provider_for_test(data_root, None).await;
        let app = test_router(&daemon);
        let app = crate::api::router(RouteHandles::from_daemon_handle(daemon.handle()));
        let app = fixture.router();
        let _raw: Store;
      }
    `,
    patterns: AUTH_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct auth-boundary global store access",
      "direct auth-boundary session store access",
      "direct auth-boundary workspace store access",
      "direct auth-boundary uncached workspace store access",
      "direct auth-boundary task store access",
      "direct auth-boundary worktree store access",
      "direct auth-boundary StoreManager access",
      "direct auth-boundary StoreManager access",
      "direct auth-boundary StoreManager access",
      "direct auth-boundary StoreManager access",
      "direct auth-boundary StoreManager global access",
      "direct auth-boundary StoreManager global access",
      "direct auth-boundary StoreManager workspace access",
      "direct auth-boundary StoreManager workspace access",
      "direct auth-boundary handle access",
      "direct auth-boundary handle access",
      "direct auth-boundary handle access",
      "direct auth-boundary handle access",
      "direct auth-boundary handle access",
      "direct auth-boundary TestDaemon construction",
      "direct auth-boundary lib-test daemon helper",
      "direct auth-boundary lib-test daemon helper",
      "direct auth-boundary router composition",
      "direct auth-boundary router composition",
      "raw auth-boundary ctx_store Store",
      "raw auth-boundary ctx_store Store",
    ],
  );
});

test("daemon boundary guard scopes auth-boundary store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/lib_tests/auth_boundaries.rs",
    "core/crates/ctx-http/src/lib_tests/auth_boundaries/daemon_http.rs",
    "core/crates/ctx-http/src/lib_tests/auth_boundaries/browser_http_bearers/basic.rs",
  ]) {
    assert.deepEqual(
      authBoundaryStorePatternsForPath(filePath),
      AUTH_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    authBoundaryStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/provider_routes/codex_routes.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects external provider-route direct store and provider facade access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/codex_login_callback_api.rs",
    contents: `
      use ctx_daemon::daemon::providers::{codex_login_status, start_codex_login_session};
      use ctx_daemon::daemon::{providers as daemon_providers};
      use ctx_daemon::daemon::{
        providers,
      };
      use ctx_daemon::daemon::providers::ProvidersHandle;
      use ctx_store::{Store, StoreManager};
      async fn helper(daemon: TestDaemon, stores: StoreManager) {
        let stores = StoreManager::open(data_dir.path()).await?;
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.stores().global().await?;
        daemon.handle().providers();
        let handle = daemon.handle();
        handle.providers();
        providers.start_codex_login_session(account_id, auth_url, None).await;
        providers.codex_login_status(account_id).await;
        let _raw: Store;
      }
    `,
    patterns: EXTERNAL_PROVIDER_ROUTE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct external provider-route global store access",
      "direct external provider-route session store access",
      "direct external provider-route workspace store access",
      "direct external provider-route StoreManager access",
      "direct external provider-route StoreManager access",
      "direct external provider-route StoreManager access",
      "direct external provider-route StoreManager access",
      "direct external provider-route provider handle access",
      "direct external provider-route provider handle access",
      "direct external provider-route provider handle access",
      "direct external provider-route provider facade import",
      "direct external provider-route provider facade import",
      "direct external provider-route provider facade import",
      "direct external provider-route provider facade import",
      "direct external provider-route provider facade import",
      "direct external provider-route codex login session API access",
      "direct external provider-route codex login session API access",
      "direct external provider-route codex login session API access",
      "raw external provider-route ctx_store Store",
      "raw external provider-route ctx_store Store",
    ],
  );
});

test("daemon boundary guard scopes external provider-route store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/codex_host_import_api.rs",
    "core/crates/ctx-http/tests/codex_login_callback_api.rs",
    "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
  ]) {
    assert.deepEqual(
      externalProviderRouteStorePatternsForPath(filePath),
      EXTERNAL_PROVIDER_ROUTE_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    externalProviderRouteStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects Gemini live catalog raw fixture setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      use common::{
        router_for_daemon as route_app,
        provider_route_providerless_daemon as providerless,
      };
      async fn helper(daemon: TestDaemon) {
        let stores = StoreManager::open(data_root).await?;
        let _raw = ctx_store::Store::open_sqlite(path, None).await?;
        Store::open_sqlite(path, None).await?;
        let app = common::router_for_daemon(&daemon);
        let app = common :: router_for_daemon(&daemon);
        let app = route_app(&daemon);
        let daemon = common::provider_route_providerless_daemon(data_root).await;
        let daemon = common :: provider_route_providerless_daemon(data_root).await;
        let daemon = providerless(data_root).await;
        let daemon = common::setup_store(data_root).await;
        let daemon = common::build_daemon(data_root, stores, providers, base_url);
        let daemon = TestDaemon::new(data_root, stores, providers, base_url, None);
        let daemon = TestDaemon :: new_with_providers_for_test(data_root, providers, base_url, None).await?;
      }
    `,
    patterns: GEMINI_LIVE_MODEL_CATALOG_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct Gemini live catalog common setup helper",
      "direct Gemini live catalog common setup helper",
      "direct Gemini live catalog common setup helper",
      "direct Gemini live catalog common setup helper",
      "direct Gemini live catalog common setup helper",
      "direct Gemini live catalog providerless daemon helper",
      "direct Gemini live catalog providerless daemon helper",
      "direct Gemini live catalog providerless daemon helper",
      "direct Gemini live catalog TestDaemon construction",
      "direct Gemini live catalog TestDaemon construction",
      "raw Gemini live catalog store setup",
      "raw Gemini live catalog store setup",
      "raw Gemini live catalog store setup",
      "raw Gemini live catalog store setup",
    ],
  );
});

test("daemon boundary guard scopes Gemini live catalog fixture root", () => {
  assert.deepEqual(
    geminiLiveModelCatalogStorePatternsForPath(
      "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
    ),
    GEMINI_LIVE_MODEL_CATALOG_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    geminiLiveModelCatalogStorePatternsForPath(
      "core/crates/ctx-http/tests/live_provider_canary.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects mobile access API storage DTO leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/mobile_access/secure.rs",
    contents: `
      use ctx_store::store::MobileAccessConfig;
      use ctx_store::store::{MobileDeviceUpsert, MobileDeviceSeqAdvance};
      use ctx_store :: store :: MobileDeviceSeqAdvance as SeqAdvance;
      async fn helper() {
        let cfg = ctx_store::store::MobileAccessConfig { id: "default".to_string() };
        let update = MobileDeviceUpsert { public_key: None };
        match outcome {
          ctx_store::store::MobileDeviceSeqAdvance::Advanced => {}
          MobileDeviceSeqAdvance::Missing => {}
          SeqAdvance::Stale { current } => {}
        }
      }
    `,
    patterns: MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "mobile access API imports storage DTOs",
      "mobile access API imports storage DTOs",
      "mobile access API imports storage DTOs",
      "mobile access API references storage DTO path",
      "mobile access API references storage DTO path",
      "mobile access API references storage DTO path",
      "mobile access API references storage DTO path",
      "mobile access API references storage DTO type",
      "mobile access API references storage DTO type",
      "mobile access API references storage DTO type",
      "mobile access API references storage DTO type",
      "mobile access API references storage DTO type",
      "mobile access API references storage DTO type",
      "mobile access API references storage DTO type",
    ],
  );
});

test("daemon boundary guard scopes mobile access storage DTO roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/mod.rs",
    "core/crates/ctx-http/src/api/mobile_access.rs",
    "core/crates/ctx-http/src/api/mobile_access/secure.rs",
    "core/crates/ctx-http/src/api/mobile_access/access_enable/profile_config.rs",
  ]) {
    assert.deepEqual(
      mobileAccessStoreDtoApiPatternsForPath(filePath),
      MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
    );
  }
  assert.deepEqual(
    mobileAccessStoreDtoApiPatternsForPath("core/crates/ctx-http/src/api/providers/status.rs"),
    [],
  );
});

test("daemon boundary guard rejects session VCS API workspace-service orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/snapshot/vcs/diff.rs",
    contents: `
      use ctx_workspace_services::worktree_vcs::{
        apply_worktree_vcs_session_patch as apply_patch,
        WorktreeVcsDiffBaseQuery,
      };
      use ctx_workspace_services::worktree_vcs as vcs;

      async fn handler() {
        let _ = ctx_workspace_services::worktree_vcs::worktree_vcs_session_diff_available("x".to_string());
        let _ = session_git_status_summary_from_snapshot(&snapshot);
        let _ = WorktreeDiffBaseResolution;
        let _ = GitStatusEntry;
      }
    `,
    patterns: SESSION_VCS_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "session VCS API imports workspace VCS service",
      "session VCS API imports workspace VCS service",
      "session VCS API imports workspace VCS service",
      "session VCS API imports workspace VCS service",
      "session VCS API aliases workspace VCS service",
      "session VCS API calls workspace VCS service helper",
      "session VCS API calls workspace VCS service helper",
      "session VCS API references workspace VCS service type",
      "session VCS API references workspace VCS service type",
      "session VCS API references workspace VCS service type",
    ],
  );
});

test("daemon boundary guard scopes session VCS orchestration patterns to API VCS roots", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/vcs.rs").includes(
      SESSION_VCS_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/vcs/apply.rs").includes(
      SESSION_VCS_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/head.rs").includes(
      SESSION_VCS_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects session model switch orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/titles_and_modes/model.rs",
    contents: `
      use ctx_provider_install::install_state::InstallTarget;
      use ctx_providers::adapters::ProviderAdapter;
      use ctx_session_tools::model_resolution::{compose_model_id, normalize_effort_id, resolve_model_id};

      async fn handler(sessions: SessionsHandle) {
        let _ = sessions.load_session_model_target_parts(session_id).await;
        let _ = sessions.ensure_provider_adapter_for_target("codex", InstallTarget::Host).await;
        let _ = sessions.load_provider_model_catalog_for_execution_environment(&workspace, "codex", env).await;
        let _ = sessions.persist_session_model_update_for_request(session_id, model, None, full).await;
        resolve_session_model_update(&sessions, &workspace, &session, env, req).await?;
        switch_live_session_model(adapter.as_ref(), &session, "model").await?;
        persist_session_model_update(&sessions, session_id, &resolved).await?;
        load_session_model_target(&sessions, session_id).await?;
      }
    `,
    patterns: SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "session model API imports model-resolution helpers directly",
      "session model API imports provider install target directly",
      "session model API imports provider install target directly",
      "session model API imports provider adapter directly",
      "session model API loads target parts directly",
      "session model API ensures provider adapter directly",
      "session model API loads provider model catalog directly",
      "session model API persists model update directly",
      "session model API defines old orchestration helpers",
      "session model API defines old orchestration helpers",
      "session model API defines old orchestration helpers",
      "session model API defines old orchestration helpers",
    ],
  );
});

test("daemon boundary guard scopes session model switch patterns to model route roots", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/titles_and_modes.rs").includes(
      SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/titles_and_modes/model.rs").includes(
      SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/sessions/titles_and_modes/model_switch.rs",
    ).includes(SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/head.rs").includes(
      SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects provider bootstrap orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/bootstrap.rs",
    contents: `
      async fn handler(providers: ProvidersHandle) {
        providers.workspace_exists(ws_id).await?;
        providers.install_target_for_workspace(ws_id).await?;
        providers.load_preferred_new_session_models(ws_id).await?;
        providers.providers_statuses_response(target, true).await;
        providers.build_bootstrap_options(ws_id, provider_status, preferred).await;
        visible_provider_count_hint(12);
        provider.detail_flag("ui_hidden");
        load_bootstrap_workspace(&providers, ws_id).await?;
        load_preferred_model_by_provider(&providers, ws_id).await?;
        load_bootstrap_accounts(&providers).await?;
        accounts::codex_accounts_response(&providers).await?;
      }
    `,
    patterns: PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider bootstrap API checks workspace existence directly",
      "provider bootstrap API resolves install target directly",
      "provider bootstrap API loads preferred models directly",
      "provider bootstrap API loads preferred models directly",
      "provider bootstrap API loads provider statuses directly",
      "provider bootstrap API builds provider options directly",
      "provider bootstrap API owns provider visibility filtering",
      "provider bootstrap API owns provider visibility filtering",
      "provider bootstrap API owns bootstrap workspace helper",
      "provider bootstrap API loads bootstrap accounts directly",
      "provider bootstrap API loads bootstrap accounts directly",
    ],
  );
});

test("daemon boundary guard scopes provider bootstrap orchestration patterns", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/bootstrap.rs").includes(
      PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/bootstrap/load.rs").includes(
      PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/status/routes.rs").includes(
      PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects provider account orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/accounts/codex.rs",
    contents: `
      async fn handler(providers: ProvidersHandle) {
        let _ = providers.load_codex_account_registry().await?;
        let _ = providers.load_codex_accounts_snapshot().await?;
        let _ = providers.ensure_amp_account_registry_from_runtime_auth().await?;
        providers.set_active_codex_account(account_id).await?;
        providers.remove_codex_account("acct").await?;
        providers.import_host_codex_auth(label).await?;
        let _ = ProviderAccountMutationError::Internal(err);
        codex_accounts_response_from_snapshot(snapshot);
        unknown_account();
        provider_account_mutation_error(err);
      }

      pub(crate) async fn amp_accounts_response(providers: &ProvidersHandle) {}
    `,
    patterns: PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider account API loads account registries directly",
      "provider account API loads account registries directly",
      "provider account API loads account registries directly",
      "provider account API mutates accounts directly",
      "provider account API mutates accounts directly",
      "provider account API mutates accounts directly",
      "provider account API matches account mutation errors directly",
      "provider account API defines local account response builders",
      "provider account API defines local account response builders",
      "provider account API owns unknown-account or mutation error mapping",
      "provider account API owns unknown-account or mutation error mapping",
    ],
  );
});

test("daemon boundary guard scopes provider account orchestration patterns", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/accounts.rs").includes(
      PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/accounts/amp.rs").includes(
      PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/bootstrap.rs").includes(
      PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects managed browser login orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
    contents: `
      #[path = "gemini/monitor.rs"]
      mod monitor;

      async fn handler(providers: ProvidersHandle) {
        tokio::spawn(async move {});
        let _request = ProviderSessionAuthenticationRequest {};
        let _device = KimiDeviceAuthorizationResp {};
        providers.authenticate_provider_session("gemini", request).await?;
        providers.prepare_gemini_login_paths(login_id).await?;
        providers.gemini_login_provider_env(login_home);
        providers.set_gemini_login_failed(login_id, error).await;
        providers.set_kimi_login_failed(login_id, error).await;
        providers.finish_gemini_login_session(login_id, account_id, None).await;
        providers.finish_kimi_login_session(login_id, account_id, None).await;
        providers.add_gemini_account_for_login(label, oauth, accounts, email).await?;
        providers.add_kimi_oauth_account_for_login(label, token, email).await?;
        providers.start_kimi_login_session(auth_url, device_code).await?;
        poll_kimi_token(device_code).await?;
        let _ = "authorization_pending";
        tokio::fs::remove_dir_all(login_home).await?;
        let _ = QWEN_OAUTH_AUTH_METHOD_ID;
      }
    `,
    patterns: MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "managed browser login API owns monitor task spawning",
      "managed browser login API declares monitor module",
      "managed browser login API constructs provider auth request",
      "managed browser login API constructs provider auth request",
      "managed browser login API owns login path or env preparation",
      "managed browser login API owns login path or env preparation",
      "managed browser login API mutates login status directly",
      "managed browser login API mutates login status directly",
      "managed browser login API mutates login status directly",
      "managed browser login API mutates login status directly",
      "managed browser login API finalizes provider accounts directly",
      "managed browser login API finalizes provider accounts directly",
      "managed browser login API owns login cleanup",
      "managed browser login API owns provider auth method constants",
      "managed browser login API owns Kimi OAuth client policy",
      "managed browser login API owns Kimi OAuth client policy",
      "managed browser login API owns Kimi OAuth protocol details",
      "managed browser login API starts Kimi sessions directly",
    ],
  );
});

test("daemon boundary guard scopes managed browser login orchestration patterns", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/browser/gemini.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/login/browser/qwen/monitor/events.rs",
    ).includes(MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/mistral/monitor.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/browser.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/kimi.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/kimi/oauth/client.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/claude/session.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects Cursor process login orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/cursor_login/session.rs",
    contents: `
      mod session;

      async fn handler(providers: ProvidersHandle) {
        tokio::spawn(async move {});
        let mut cmd = tokio::process::Command::new("cursor-agent");
        cmd.stdin(Stdio::null());
        let _ = providers.resolve_cursor_login_runtime().await?;
        providers.start_cursor_login_session().await;
        providers.set_cursor_login_error(login_id, error).await;
        providers.update_cursor_login_auth_url(login_id, auth_url).await;
        providers.finish_cursor_login_session(login_id, status, account_id, error, auth_url).await;
        providers.add_cursor_oauth_account_for_login(label, token, refresh, email).await?;
        let _home = cursor_login_home(data_root, login_id);
        ensure_private_dir(&_home).await?;
        initialize_cursor_capture_file(path).await?;
        write_cursor_capture_hook(path).await?;
        let _ = CTX_CURSOR_CAPTURE_FILE;
        let _ = NODE_OPTIONS;
        parse_cursor_captured_tokens(path).await?;
        spawn_cursor_login_reader(stdout, false, tx);
        collect_cursor_login_output(providers, login_id, child).await;
        record_cursor_login_output(providers, login_id, line, transcript, email, auth_url).await;
        let _ = first_email_from_text("dev@example.com");
        let _line = CursorLoginOutputLine {};
        let _ = CTX_CURSOR_LOGIN_TIMEOUT_SECS;
        let _ = cursor_login_timeout();
        for key in DAEMON_AUTH_ENV_VARS {}
      }
    `,
    patterns: CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "Cursor process login API owns monitor task spawning",
      "Cursor process login API owns process spawning",
      "Cursor process login API owns process spawning",
      "Cursor process login API resolves runtime directly",
      "Cursor process login API mutates login sessions directly",
      "Cursor process login API mutates login sessions directly",
      "Cursor process login API mutates login sessions directly",
      "Cursor process login API mutates login sessions directly",
      "Cursor process login API finalizes Cursor accounts directly",
      "Cursor process login API owns private capture workspace",
      "Cursor process login API owns private capture workspace",
      "Cursor process login API owns private capture workspace",
      "Cursor process login API owns private capture workspace",
      "Cursor process login API owns private capture workspace",
      "Cursor process login API owns private capture workspace",
      "Cursor process login API owns process output parsing",
      "Cursor process login API owns process output parsing",
      "Cursor process login API owns process output parsing",
      "Cursor process login API owns process output parsing",
      "Cursor process login API owns process output parsing",
      "Cursor process login API owns process output parsing",
      "Cursor process login API owns login timeout or env scrubbing",
      "Cursor process login API owns login timeout or env scrubbing",
      "Cursor process login API owns login timeout or env scrubbing",
      "Cursor process login API declares process monitor module",
    ],
  );
});

test("daemon boundary guard scopes Cursor process login orchestration patterns", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/cursor_login.rs").includes(
      CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/cursor_login/session/output_loop.rs",
    ).includes(CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/codex.rs").includes(
      CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects Codex app-server login orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/codex.rs",
    contents: `
      mod app_server;
      mod completion;
      mod process;

      async fn handler(providers: ProvidersHandle) {
        tokio::spawn(async move {});
        let mut cmd = tokio::process::Command::new("codex");
        cmd.stdin(Stdio::null());
        let _ = CODEX_APP_SERVER_ARGS;
        let _ = CODEX_LOGIN_RPC_TIMEOUT;
        for key in DAEMON_AUTH_ENV_VARS {}
        send_codex_jsonrpc(&mut stdin, &request).await?;
        wait_for_codex_response(&mut reader, 1, timeout).await?;
        spawn_codex_app_server(dir, bin)?;
        start_codex_login_process(dir, bin).await?;
        monitor_codex_login(providers.clone(), account_id, label, login).await;
        wait_for_codex_login_completion(&mut reader, login_id).await?;
        fetch_codex_account_details(&mut stdin, &mut reader).await?;
        providers.prepare_codex_login_start(label).await?;
        providers.start_codex_login_session(account_id, auth_url, expected).await;
        providers.claim_codex_login_callback(account_id, token).await?;
        providers.restore_codex_login_completion_token(account_id, token).await;
        providers.finish_codex_login_session(account_id, true, None).await;
        providers.persist_successful_codex_login(account_id, label, None, None).await?;
        let _ = reqwest::Client::builder();
        callback_replay_client(callback_url)?;
        replay_codex_callback(callback_url).await?;
        let _err = CallbackReplayError::Request("boom".into());
        remove_dir_all(account_dir).await?;
      }
    `,
    patterns: CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "Codex app-server login API owns monitor task spawning",
      "Codex app-server login API declares app-server modules",
      "Codex app-server login API declares app-server modules",
      "Codex app-server login API declares app-server modules",
      "Codex app-server login API owns process spawning",
      "Codex app-server login API owns process spawning",
      "Codex app-server login API owns app-server runtime policy",
      "Codex app-server login API owns app-server runtime policy",
      "Codex app-server login API owns app-server runtime policy",
      "Codex app-server login API owns app-server JSON-RPC",
      "Codex app-server login API owns app-server JSON-RPC",
      "Codex app-server login API owns app-server JSON-RPC",
      "Codex app-server login API owns app-server JSON-RPC",
      "Codex app-server login API owns app-server JSON-RPC",
      "Codex app-server login API owns app-server JSON-RPC",
      "Codex app-server login API owns app-server JSON-RPC",
      "Codex app-server login API mutates login sessions directly",
      "Codex app-server login API mutates login sessions directly",
      "Codex app-server login API mutates login sessions directly",
      "Codex app-server login API mutates login sessions directly",
      "Codex app-server login API mutates login sessions directly",
      "Codex app-server login API finalizes Codex accounts directly",
      "Codex app-server login API owns callback replay",
      "Codex app-server login API owns callback replay",
      "Codex app-server login API owns callback replay",
      "Codex app-server login API owns callback replay",
      "Codex app-server login API owns login cleanup",
    ],
  );
});

test("daemon boundary guard scopes Codex app-server login orchestration patterns", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login.rs").includes(
      CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/codex.rs").includes(
      CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/login/codex/app_server.rs",
    ).includes(CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/cursor_login.rs").includes(
      CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects task-session API admission ownership", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/tasks/creation_session/create.rs",
    contents: `
      async fn handler(
        State(_sessions): State<SessionsHandle>,
        State(sessions): State<ctx_daemon::daemon::SessionsHandle>,
      ) {
        let creation_lock = sessions.task_session_creation_lock(task_id).await;
        let _ = tasks.create_session_for_loaded_task(store, task, workspace, input).await;
      }
    `,
    patterns: TASK_SESSION_CREATION_API_ADMISSION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "task session API owns sessions handle",
      "task session API owns sessions handle",
      "task session API owns session creation lock",
      "task session API bypasses locked create-session entrypoint",
    ],
  );
});

test("daemon boundary guard scopes task-session admission patterns to creation-session roots", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/tasks/creation_session/create.rs").includes(
      TASK_SESSION_CREATION_API_ADMISSION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/tasks/creation_session.rs").includes(
      TASK_SESSION_CREATION_API_ADMISSION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/tasks/task_deletion.rs").includes(
      TASK_SESSION_CREATION_API_ADMISSION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects small API unit direct store setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/codex/tests.rs",
    contents: `
      use ctx_settings_service::save_settings;
      use ctx_settings_service::{
        load_settings,
        save_settings as persist_settings,
      };
      use ctx_store::{Store, StoreManager};
      use ctx_store::{
        Store as RawStore,
      };
      async fn helper(daemon: TestDaemon) {
        let stores = StoreManager::open(data_dir.path()).await?;
        daemon.stores().global().await?;
        daemon.global_store();
        let daemon = TestDaemon::new(data_dir, stores, providers, "http://127.0.0.1:0".into(), None);
        let daemon = TestDaemon::new_for_test(data_root, "http://127.0.0.1:4310".to_string()).await?;
        let _app = crate::api::router(RouteHandles::from_daemon_handle(daemon.handle()));
        ctx_settings_service::save_settings(daemon.global_store(), &settings).await?;
        save_settings(daemon.global_store(), &settings).await?;
        persist_settings(daemon.global_store(), &settings).await?;
        let _raw = ctx_store::Store::open_sqlite(path, None).await?;
        Store::open_sqlite(path, None).await?;
        RawStore::open_sqlite(path, None).await?;
        daemon.handle().providers().persist_successful_codex_login(account_id, label, None, None).await?;
        daemon
          .handle()
          .providers()
          .persist_successful_codex_login(account_id, label, None, None)
          .await?;
        let Json(resp) = get_install_statuses(
          State(daemon.handle().providers()),
          Json(req),
        ).await?;
        get_install_statuses(
          State(other), State(daemon.handle().providers()),
          Json(req),
        ).await?;
        get_install_statuses(foo, State(daemon.handle().providers()), Json(req)).await?;
        let bypass = State(daemon.handle().providers());
        let handle = daemon.handle();
        handle.providers();
        update_settings(State(daemon.handle().core()), Json(req)).await?;
        update_settings(State(handle.core()), Json(req)).await?;
        let Json(resp) = get_install_statuses(
          State(handle.providers()),
          Json(req),
        ).await?;
        let label = "Secret Store";
      }
    `,
    patterns: SMALL_API_UNIT_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct small API unit StoreManager access",
      "direct small API unit StoreManager access",
      "direct small API unit StoreManager access",
      "raw small API unit ctx_store Store",
      "raw small API unit ctx_store Store",
      "raw small API unit ctx_store Store",
      "raw small API unit ctx_store Store",
      "direct small API unit provider handle reach-through",
      "direct small API unit provider handle reach-through",
      "direct small API unit provider handle reach-through",
      "direct small API unit provider handle reach-through",
      "direct small API unit provider handle reach-through",
      "direct small API unit provider handle reach-through",
      "direct small API unit provider handle reach-through",
      "direct small API unit provider handle reach-through",
      "direct small API unit core handle reach-through",
      "direct small API unit core handle reach-through",
      "direct small API unit raw TestDaemon construction",
      "direct small API unit raw TestDaemon construction",
      "direct small API unit router composition",
      "direct small API unit global store access",
      "direct small API unit global store access",
      "direct small API unit global store access",
      "direct small API unit global store access",
      "direct small API unit settings persistence",
      "direct small API unit settings persistence",
      "direct small API unit settings persistence",
      "direct small API unit settings persistence",
    ],
  );
});

test("daemon boundary guard allows small API unit test daemon facades and handler state", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/settings.rs",
    contents: `
      async fn helper() {
        let fixture = crate::test_support::TestDaemonFixture::new("http://127.0.0.1:4310").await;
        update_settings(State(fixture.core()), Json(req)).await?;
        get_install_statuses(State(fixture.providers()), Json(req)).await?;
        fixture.providers().restart_provider_for_auth_change("codex", "test").await?;
      }
    `,
    patterns: SMALL_API_UNIT_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes small API unit store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/settings.rs",
    "core/crates/ctx-http/src/api/providers/login/codex/tests.rs",
    "core/crates/ctx-http/src/api/providers/tests/mod.rs",
    "core/crates/ctx-http/src/api/providers/tests/install_statuses.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/fixtures.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/failures.rs",
    "core/crates/ctx-http/src/api/providers/tests/restarts/harness_source.rs",
    "core/crates/ctx-http/src/api/sessions/tests.rs",
    "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
    "core/crates/ctx-http/src/api/workspaces/tests.rs",
  ]) {
    assert.deepEqual(
      smallApiUnitStorePatternsForPath(filePath),
      SMALL_API_UNIT_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    smallApiUnitStorePatternsForPath(
      "core/crates/ctx-http/src/api/providers/tests/restarts/support.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects small external direct store setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      use ctx_store::{
        Store as RawStore,
      };
      use common::{setup_store, build_daemon};
      use common::{
        setup_store as setup_store_alias,
        build_daemon as build_daemon_alias,
      };
      use crate::common::{
        setup_store as setup_store_crate_alias,
        build_daemon as build_daemon_crate_alias,
      };
      use crate::{
        common::setup_store as setup_store_root_alias,
        common::build_daemon as build_daemon_root_alias,
      };
      async fn helper(daemon: TestDaemon) {
        let stores = common::setup_store(data_dir.path()).await;
        let stores = crate::common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_root, stores, common::fake_providers(), base_url);
        let daemon = crate::common::build_daemon(data_root, stores, common::fake_providers(), base_url);
        let stores = setup_store(data_dir.path()).await;
        let daemon = build_daemon(data_root, stores, common::fake_providers(), base_url);
        let stores = setup_store_alias(data_dir.path()).await;
        let daemon = build_daemon_alias(data_root, stores, common::fake_providers(), base_url);
        let stores = setup_store_crate_alias(data_dir.path()).await;
        let daemon = build_daemon_crate_alias(data_root, stores, common::fake_providers(), base_url);
        let stores = setup_store_root_alias(data_dir.path()).await;
        let daemon = build_daemon_root_alias(data_root, stores, common::fake_providers(), base_url);
        let _stores = StoreManager::open(data_root).await?;
        let _raw = ctx_store::Store::open_sqlite(path, None).await?;
        Store::open_sqlite(path, None).await?;
        RawStore::open_sqlite(path, None).await?;
        let daemon = TestDaemon::new_with_providers_for_test(data_root, providers, base_url, None).await?;
        use ctx_daemon::test_support::TestDaemon as RawDaemon;
        let daemon = RawDaemon::new_with_providers_for_test(data_root, providers, base_url, None).await?;
        use ctx_daemon::test_support::{Other, TestDaemon as GroupedRawDaemon};
        let daemon = GroupedRawDaemon::new_for_test(data_root, base_url).await?;
        type DaemonAlias = TestDaemon;
        let daemon = DaemonAlias::new_for_test(data_root, base_url).await?;
        type QualifiedDaemonAlias = ctx_daemon::test_support::TestDaemon;
        let daemon = QualifiedDaemonAlias::new_for_test(data_root, base_url).await?;
        daemon.stores().global().await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
      }
    `,
    patterns: SMALL_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct small external StoreManager access",
      "direct small external StoreManager access",
      "raw small external ctx_store Store",
      "raw small external ctx_store Store",
      "raw small external ctx_store Store",
      "raw small external ctx_store Store",
      "direct small external store manager helper",
      "direct small external store manager helper",
      "direct small external store manager helper",
      "direct small external store manager helper",
      "direct small external store manager helper",
      "direct small external store manager helper",
      "direct small external store manager helper",
      "direct small external store manager helper",
      "direct small external daemon construction helper",
      "direct small external daemon construction helper",
      "direct small external daemon construction helper",
      "direct small external daemon construction helper",
      "direct small external daemon construction helper",
      "direct small external daemon construction helper",
      "direct small external daemon construction helper",
      "direct small external daemon construction helper",
      "direct small external TestDaemon construction",
      "direct small external TestDaemon construction",
      "direct small external TestDaemon construction",
      "direct small external TestDaemon construction",
      "direct small external TestDaemon construction",
      "direct small external TestDaemon store access",
      "direct small external TestDaemon store access",
      "direct small external TestDaemon store access",
    ],
  );
});

test("daemon boundary guard allows small external data-root fake-daemon fixture", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/title_generation_local_e2e.rs",
    contents: `
      async fn helper(data_root: &Path) {
        let fixture = common::fake_daemon_fixture_for_data_root(data_root, "http://127.0.0.1:0").await;
        let state = &fixture.daemon;
        state.start_install("title_generation_local".to_string(), None).await;
      }
    `,
    patterns: SMALL_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes small external store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
    "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
    "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
    "core/crates/ctx-http/tests/repo_validate_destination.rs",
    "core/crates/ctx-http/tests/system_prompt_append_http.rs",
    "core/crates/ctx-http/tests/title_generation_local_e2e.rs",
    "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
    "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
  ]) {
    assert.deepEqual(
      smallExternalStorePatternsForPath(filePath),
      SMALL_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    smallExternalStorePatternsForPath("core/crates/ctx-http/tests/title_generation_local.rs"),
    [],
  );
});

test("daemon boundary guard rejects fake-daemon external raw setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/demo_seed_transcript_http.rs",
    contents: `
      async fn helper(daemon: TestDaemon) {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_root, stores, common::fake_providers(), base_url);
        let _stores = StoreManager::open(data_root).await?;
        let _raw = ctx_store::Store::open_sqlite(path, None).await?;
        let daemon = TestDaemon::new(data_root, stores, common::fake_providers(), base_url, None);
        let daemon = TestDaemon::new_with_providers_for_test(data_root, common::fake_providers(), base_url, None).await?;
        daemon.stores().global().await?;
        daemon.store_for_session(session_id).await?;
      }
    `,
    patterns: FAKE_DAEMON_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct fake-daemon external StoreManager access",
      "raw fake-daemon external ctx_store Store",
      "direct fake-daemon external store manager helper",
      "direct fake-daemon external daemon construction helper",
      "direct fake-daemon external TestDaemon construction",
      "direct fake-daemon external TestDaemon construction",
      "direct fake-daemon external TestDaemon store access",
      "direct fake-daemon external TestDaemon store access",
    ],
  );
});

test("daemon boundary guard scopes fake-daemon external roots without blocking common helper", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/acp_target_scoped_status.rs",
    "core/crates/ctx-http/tests/assistant_message_persistence_faults.rs",
    "core/crates/ctx-http/tests/cache_rehydration.rs",
    "core/crates/ctx-http/tests/demo_seed_transcript_http.rs",
    "core/crates/ctx-http/tests/fault_matrix.rs",
    "core/crates/ctx-http/tests/global_id_routing_http.rs",
    "core/crates/ctx-http/tests/hot_endpoints_no_db.rs",
    "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
    "core/crates/ctx-http/tests/install_start_contract.rs",
    "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    "core/crates/ctx-http/tests/merge_queue_isolation.rs",
    "core/crates/ctx-http/tests/memory_leak_e2e.rs",
    "core/crates/ctx-http/tests/message_idempotency.rs",
    "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
    "core/crates/ctx-http/tests/provider_current_ctx_version_regressions.rs",
    "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
    "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
    "core/crates/ctx-http/tests/session_model_api.rs",
    "core/crates/ctx-http/tests/subscription_accounts_api.rs",
    "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
    "core/crates/ctx-http/tests/terminal_ws_reconnect.rs",
    "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
    "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    "core/crates/ctx-http/tests/workspace_attachments_local_canonical.rs",
    "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
    "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
    "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
    "core/crates/ctx-http/tests/workspace_stream_stress_active_heads_lag.rs",
    "core/crates/ctx-http/tests/worktree_archive_http.rs",
    "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
  ]) {
    assert.deepEqual(
      fakeDaemonExternalStorePatternsForPath(filePath),
      FAKE_DAEMON_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    fakeDaemonExternalStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects ACP CRP bridge token raw settings store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/acp_crp_bridge_tokens_e2e.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      use ctx_store :: Store as SpacedStore;
      use ctx_settings_service::{load_settings, load_settings_from_data_root};
      use ctx_settings_service :: load_settings as spaced_load_settings;
      async fn helper(daemon: TestDaemon, stores: StoreManager) {
        let _stores = StoreManager::open(data_root).await?;
        let _raw = ctx_store::Store::open_sqlite(path, None).await?;
        let store = Store::open_sqlite(path, Some(1)).await?;
        let _spaced = ctx_store :: Store :: open_sqlite(path, None).await?;
        let _aliased = SpacedStore :: open_sqlite(path, None).await?;
        let settings = ctx_settings_service::load_settings(&store).await?;
        let settings = ctx_settings_service :: load_settings(&store).await?;
        let settings = load_settings(&store).await?;
        let settings = spaced_load_settings(&store).await?;
        let settings = load_settings_from_data_root(data_root).await?;
        let db_path = data_root.join("db").join("db.sqlite");
        daemon.stores().global().await?;
        daemon.global_store().get_workspace(workspace_id).await?;
        daemon.store_for_session(session_id).await?;
        client.close().await;
        store.close().await;
      }
    `,
    patterns: ACP_CRP_BRIDGE_TOKEN_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct ACP CRP bridge token StoreManager access",
      "direct ACP CRP bridge token StoreManager access",
      "direct ACP CRP bridge token StoreManager access",
      "raw ACP CRP bridge token ctx_store Store",
      "raw ACP CRP bridge token ctx_store Store",
      "raw ACP CRP bridge token ctx_store Store",
      "raw ACP CRP bridge token ctx_store Store",
      "raw ACP CRP bridge token ctx_store Store",
      "raw ACP CRP bridge token ctx_store Store",
      "direct ACP CRP bridge token TestDaemon store access",
      "direct ACP CRP bridge token TestDaemon store access",
      "direct ACP CRP bridge token TestDaemon store access",
      "direct ACP CRP bridge token settings store load",
      "direct ACP CRP bridge token settings store load",
      "direct ACP CRP bridge token settings store load",
      "direct ACP CRP bridge token settings store load",
      "direct ACP CRP bridge token settings store load",
      "direct ACP CRP bridge token settings DB path",
      "direct ACP CRP bridge token settings store close",
    ],
  );
});

test("daemon boundary guard scopes ACP CRP bridge token store facade root", () => {
  assert.deepEqual(
    acpCrpBridgeTokenStorePatternsForPath(
      "core/crates/ctx-http/tests/acp_crp_bridge_tokens_e2e.rs",
    ),
    ACP_CRP_BRIDGE_TOKEN_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    acpCrpBridgeTokenStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects image attachments direct store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
    contents: `
      async fn helper(daemon: TestDaemon) {
        let app = common::router_for_daemon(&daemon);
        daemon.global_store().insert_blob(&blob_id, &sha, bytes, "image/png", None, now).await?;
        daemon.store_for_session(session_id).await?.count_user_messages_for_session(session_id).await?;
        daemon.store_for_session(session_id).await?.list_messages_for_session(session_id).await?;
      }
    `,
    patterns: IMAGE_ATTACHMENTS_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct image attachments daemon router composition",
      "direct image attachments generic store access",
      "direct image attachments generic store access",
      "direct image attachments generic store access",
      "direct image attachments blob store access",
      "direct image attachments session message query",
      "direct image attachments session message query",
    ],
  );
});

test("daemon boundary guard scopes image attachments store facade root", () => {
  assert.deepEqual(
    imageAttachmentsStorePatternsForPath(
      "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
    ),
    IMAGE_ATTACHMENTS_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    imageAttachmentsStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects workspace attachments demo raw setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/attachments_demo_react.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      use common::{setup_store, build_daemon, router_for_daemon};
      async fn helper(daemon: TestDaemon) {
        let stores = StoreManager::open(data_root).await?;
        let _stores = common::setup_store(data_root).await;
        let _stores = setup_store(data_root).await;
        let _daemon = TestDaemon::new(data_root, stores, providers, base_url, None);
        let _daemon = common::build_daemon(data_root, stores, providers, base_url);
        let _daemon = build_daemon(data_root, stores, providers, base_url);
        let _app = common::router_for_daemon(&daemon);
        let _app = router_for_daemon(&daemon);
        daemon.global_store().create_workspace("demo".into(), root.into(), VcsKind::Git).await?;
        daemon.store_for_workspace(workspace_id).await?;
        let worktree = Worktree { id, workspace_id, root_path, base_commit_sha, git_branch: None };
        store.insert_worktree(worktree).await?;
        store.create_worktree(workspace_id, root_path, base_commit_sha, None).await?;
        daemon.seed_workspace_for_test("demo", root, VcsKind::Git).await?;
        daemon.seed_task_lifecycle_workspace_for_test("demo", root, VcsKind::Git).await?;
        daemon.seed_task_lifecycle_task_for_test(workspace_id, "demo").await?;
        let _raw = ctx_store::Store::open_sqlite(path, None).await?;
        Store::open_sqlite(path, None).await?;
      }
    `,
    patterns: WORKSPACE_ATTACHMENTS_DEMO_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct workspace attachments demo StoreManager access",
      "direct workspace attachments demo StoreManager access",
      "raw workspace attachments demo ctx_store Store",
      "raw workspace attachments demo ctx_store Store",
      "raw workspace attachments demo ctx_store Store",
      "direct workspace attachments demo raw TestDaemon construction",
      "direct workspace attachments demo store setup helper",
      "direct workspace attachments demo store setup helper",
      "direct workspace attachments demo store setup helper",
      "direct workspace attachments demo store setup helper",
      "direct workspace attachments demo daemon construction helper",
      "direct workspace attachments demo daemon construction helper",
      "direct workspace attachments demo daemon construction helper",
      "direct workspace attachments demo daemon construction helper",
      "direct workspace attachments demo daemon router composition",
      "direct workspace attachments demo daemon router composition",
      "direct workspace attachments demo daemon router composition",
      "direct workspace attachments demo daemon router composition",
      "direct workspace attachments demo generic store access",
      "direct workspace attachments demo generic store access",
      "direct workspace attachments demo manual worktree setup",
      "direct workspace attachments demo manual worktree setup",
      "direct workspace attachments demo manual worktree setup",
      "direct workspace attachments demo generic seed helper",
      "direct workspace attachments demo generic seed helper",
      "direct workspace attachments demo generic seed helper",
    ],
  );
});

test("daemon boundary guard allows workspace attachments demo fixture path", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/attachments_demo_react.rs",
    contents: `
      async fn helper(data_root: &Path, ws_root: &Path) {
        let fixture = common::fake_daemon_fixture_for_data_root(data_root, "http://127.0.0.1:0").await;
        let seeded = fixture.daemon.seed_workspace_attachments_demo_fixture_for_test("demo", ws_root, base_commit).await?;
        fixture.daemon.materialize_workspace_attachments_for_test(&seeded.workspace, &seeded.worktree, configs).await?;
      }
    `,
    patterns: WORKSPACE_ATTACHMENTS_DEMO_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes workspace attachments demo root", () => {
  assert.deepEqual(
    workspaceAttachmentsDemoStorePatternsForPath(
      "core/crates/ctx-http/tests/attachments_demo_react.rs",
    ),
    WORKSPACE_ATTACHMENTS_DEMO_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    workspaceAttachmentsDemoStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects provider target scoped install raw setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      use common::{setup_store, build_daemon, router_for_daemon};
      async fn helper(daemon: TestDaemon) {
        let stores = StoreManager::open(data_root).await?;
        let _stores = common::setup_store(data_root).await;
        let _stores = setup_store(data_root).await;
        let _daemon = common::build_daemon(data_root, stores, providers, base_url);
        let _daemon = build_daemon(data_root, stores, providers, base_url);
        let _app = common::router_for_daemon(&daemon);
        let _app = router_for_daemon(&daemon);
        let _raw = ctx_store::Store::open_sqlite(path, None).await?;
        Store::open_sqlite(path, None).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.store_for_session(session_id).await?;
        daemon.get_install_info(install_id).await;
        daemon.find_running_install("kimi", target).await;
        daemon.tracked_install_ids("kimi", target).await;
        daemon.has_target_provider_adapter("cache-key").await;
        daemon.target_provider_adapter_cache_keys().await;
        daemon.start_install("kimi".to_string(), target).await;
        daemon.install_provider_with_progress(install_id, "kimi".to_string(), target).await?;
      }
    `,
    patterns: PROVIDER_TARGET_SCOPED_INSTALLS_TEST_STORE_ACCESS_PATTERNS,
  });
  const names = new Set(violations.map((violation) => violation.name));

  for (const name of [
    "direct provider target scoped installs StoreManager access",
    "raw provider target scoped installs ctx_store Store",
    "direct provider target scoped installs store setup helper",
    "direct provider target scoped installs daemon construction helper",
    "direct provider target scoped installs daemon router composition",
    "direct provider target scoped installs generic store access",
    "direct provider target scoped installs broad install/cache observation",
  ]) {
    assert.ok(names.has(name), `expected violation ${name}; got ${[...names].join(", ")}`);
  }
});

test("daemon boundary guard allows provider target scoped install fixtures", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
    contents: `
      async fn helper(data_root: &Path, daemon: &TestDaemon) {
        let fixture = providerless_install_fixture(data_root).await;
        let app = fixture.router();
        let install_info = daemon.provider_target_install_info_for_test(install_id).await;
        let ids = daemon.provider_target_tracked_install_ids_for_test("kimi", target).await;
        daemon.provider_target_has_adapter_cache_entry_for_test("cache-key").await;
        daemon.provider_target_adapter_cache_keys_for_test().await;
        daemon.provider_target_start_tracked_install_for_test("kimi".to_string(), target).await;
        daemon.provider_target_install_with_progress_for_test(install_id, "kimi".to_string(), target).await?;
        daemon.write_workspace_container_execution_without_runtime_probe_for_test(workspace_id).await?;
        daemon.provider_target_session_events_after_done_for_test(session_id, "done", timeout).await?;
      }
    `,
    patterns: PROVIDER_TARGET_SCOPED_INSTALLS_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes provider target scoped install root", () => {
  assert.deepEqual(
    providerTargetScopedInstallsStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
    ),
    PROVIDER_TARGET_SCOPED_INSTALLS_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    providerTargetScopedInstallsStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects worktree-vcs daemon router composition", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
    contents: `
      async fn helper(daemon: TestDaemon) {
        let app = common::router_for_daemon(&daemon);
      }
    `,
    patterns: WORKTREE_VCS_SNAPSHOT_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["direct worktree-vcs-snapshot daemon router composition"],
  );
});

test("daemon boundary guard allows worktree-vcs daemon facades", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
    contents: `
      async fn helper(daemon: TestDaemon, worktree: Worktree) {
        daemon.load_worktree_for_test(session.worktree_id).await?;
        daemon.mark_worktree_vcs_active_for_test(worktree.id).await;
        daemon.emit_worktree_vcs_snapshot_for_worktree(&worktree, true).await?;
        daemon.request_worktree_vcs_refresh_for_test(&worktree, true, true).await?;
        daemon.mark_worktree_vcs_filesystem_dirty_for_test(&worktree, "file.txt").await?;
        daemon.mark_worktree_vcs_metadata_dirty_for_test(&worktree, ".git/HEAD").await?;
        daemon.run_git_status_watcher_for_test(worktree.clone()).await?;
        daemon.worktree_vcs_snapshot(worktree.id).await;
      }
    `,
    patterns: WORKTREE_VCS_SNAPSHOT_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes worktree-vcs store facade root", () => {
  assert.deepEqual(
    worktreeVcsSnapshotStorePatternsForPath(
      "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
    ),
    WORKTREE_VCS_SNAPSHOT_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    worktreeVcsSnapshotStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects provider-probe daemon router composition", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
    contents: `
      async fn helper(daemon: TestDaemon) {
        let app = common::router_for_daemon(&daemon);
      }
    `,
    patterns: PROVIDER_PROBE_RUNTIME_ENV_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["direct provider-probe daemon router composition"],
  );
});

test("daemon boundary guard scopes provider-probe runtime env root", () => {
  assert.deepEqual(
    providerProbeRuntimeEnvStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
    ),
    PROVIDER_PROBE_RUNTIME_ENV_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    providerProbeRuntimeEnvStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects default-session/diff raw daemon fixture setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/task_default_session_http.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn helper(daemon: &TestDaemon) {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
        let app = common::router_for_daemon(&daemon);
        let daemon = TestDaemon::new(data_dir, stores, providers, "http://127.0.0.1:0".into(), None);
        daemon.store_for_workspace(workspace_id).await?;
      }
    `,
    patterns: DEFAULT_SESSION_AND_DIFF_FAKE_DAEMON_FIXTURE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct default-session/diff store manager helper",
      "direct default-session/diff daemon construction helper",
      "direct default-session/diff daemon router composition",
      "direct default-session/diff TestDaemon construction",
      "direct default-session/diff TestDaemon store access",
      "raw default-session/diff StoreManager",
      "raw default-session/diff ctx_store Store",
    ],
  );
});

test("daemon boundary guard allows default-session/diff fake-daemon fixture router", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/session_diff_unavailable.rs",
    contents: `
      async fn helper() {
        let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
        let state = &fixture.daemon;
        let app = fixture.router();
        state.seed_workspace_runtime_settings_without_target_branch_for_test(workspace_id).await?;
      }
    `,
    patterns: DEFAULT_SESSION_AND_DIFF_FAKE_DAEMON_FIXTURE_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes default-session/diff fake-daemon fixture roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/session_diff_unavailable.rs",
    "core/crates/ctx-http/tests/task_default_session_http.rs",
  ]) {
    assert.deepEqual(
      defaultSessionAndDiffFakeDaemonFixturePatternsForPath(filePath),
      DEFAULT_SESSION_AND_DIFF_FAKE_DAEMON_FIXTURE_PATTERNS,
    );
  }
  assert.deepEqual(
    defaultSessionAndDiffFakeDaemonFixturePatternsForPath(
      "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects workspace/VCS raw daemon fixture setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/workspace_active_snapshot_http.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn helper(daemon: &TestDaemon) {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
        let app = common::router_for_daemon(&daemon);
        let daemon = TestDaemon::new(data_dir, stores, providers, "http://127.0.0.1:0".into(), None);
        daemon.store_for_workspace(workspace_id).await?;
      }
    `,
    patterns: WORKSPACE_VCS_SETUP_FIXTURE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct workspace/VCS setup store manager helper",
      "direct workspace/VCS setup daemon construction helper",
      "direct workspace/VCS setup daemon router composition",
      "direct workspace/VCS setup TestDaemon construction",
      "direct workspace/VCS setup TestDaemon store access",
      "raw workspace/VCS setup StoreManager",
      "raw workspace/VCS setup ctx_store Store",
    ],
  );
});

test("daemon boundary guard allows workspace/VCS fake-daemon fixture router", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/disk_isolated_sandbox_smoke.rs",
    contents: `
      async fn helper() {
        let fixture = common::fake_daemon_fixture_with_providers(
          common::fake_providers(),
          "http://127.0.0.1:4399",
        ).await;
        let app = fixture.router();
        let data_root = fixture.data_dir.path();
        assert!(sandbox_volume_exists(data_root, &vol_name).await);
      }
    `,
    patterns: WORKSPACE_VCS_SETUP_FIXTURE_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes workspace/VCS fixture roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/workspace_active_snapshot_http.rs",
    "core/crates/ctx-http/tests/disk_isolated_sandbox_smoke.rs",
    "core/crates/ctx-http/tests/disk_isolated_vcs_integrity.rs",
  ]) {
    assert.deepEqual(
      workspaceVcsSetupFixturePatternsForPath(filePath),
      WORKSPACE_VCS_SETUP_FIXTURE_PATTERNS,
    );
  }
  assert.deepEqual(
    workspaceVcsSetupFixturePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects small-route raw daemon fixture setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/repo_validate_destination.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      use common::{
        provider_route_fake_daemon as fake_daemon,
        router_for_daemon as router,
      };
      async fn helper(daemon: &TestDaemon) {
        let daemon = common::provider_route_fake_daemon(data_dir.path()).await;
        let app = common::router_for_daemon(&daemon);
        let daemon = provider_route_fake_daemon(data_dir.path()).await;
        let app = router_for_daemon(&daemon);
        let app = router(&daemon);
        let daemon = fake_daemon(data_dir.path()).await;
        let daemon = TestDaemon::new(data_dir, stores, providers, "http://127.0.0.1:0".into(), None);
        daemon.store_for_workspace(workspace_id).await?;
      }
    `,
    patterns: SMALL_ROUTE_FIXTURE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct small-route provider daemon fixture helper",
      "direct small-route provider daemon fixture helper",
      "direct small-route provider daemon fixture helper",
      "direct small-route daemon router composition",
      "direct small-route daemon router composition",
      "direct small-route daemon router composition",
      "direct small-route TestDaemon construction",
      "direct small-route TestDaemon store access",
      "raw small-route StoreManager",
      "raw small-route ctx_store Store",
    ],
  );
});

test("daemon boundary guard allows small-route fake-daemon fixture router", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    contents: `
      async fn helper() {
        let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
        let daemon = &fixture.daemon;
        let app = fixture.router();
        daemon.seed_workspace_merge_queue_queued_entry_for_test(workspace_id, "entry").await?;
      }
    `,
    patterns: SMALL_ROUTE_FIXTURE_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes small-route fixture roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
    "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
    "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
    "core/crates/ctx-http/tests/repo_validate_destination.rs",
    "core/crates/ctx-http/tests/system_prompt_append_http.rs",
    "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
    "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
  ]) {
    assert.deepEqual(
      smallRouteFixturePatternsForPath(filePath),
      SMALL_ROUTE_FIXTURE_PATTERNS,
    );
  }
  assert.deepEqual(
    smallRouteFixturePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects provider-auth/global-id raw daemon fixture setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/codex_login_callback_api.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      use common::{
        provider_route_fake_daemon as fake_daemon,
        router_for_daemon as router,
      };
      async fn helper(daemon: &TestDaemon) {
        let daemon = common::provider_route_fake_daemon(data_dir.path()).await;
        let app = common::router_for_daemon(&daemon);
        let daemon = provider_route_fake_daemon(data_dir.path()).await;
        let app = router_for_daemon(&daemon);
        let app = router(&daemon);
        let daemon = fake_daemon(data_dir.path()).await;
        let daemon = TestDaemon::new(data_dir, stores, providers, "http://127.0.0.1:0".into(), None);
        daemon.store_for_session(session_id).await?;
      }
    `,
    patterns: PROVIDER_AUTH_GLOBAL_ID_FIXTURE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct provider-auth/global-id provider daemon fixture helper",
      "direct provider-auth/global-id provider daemon fixture helper",
      "direct provider-auth/global-id provider daemon fixture helper",
      "direct provider-auth/global-id daemon router composition",
      "direct provider-auth/global-id daemon router composition",
      "direct provider-auth/global-id daemon router composition",
      "direct provider-auth/global-id TestDaemon construction",
      "direct provider-auth/global-id TestDaemon store access",
      "raw provider-auth/global-id StoreManager",
      "raw provider-auth/global-id ctx_store Store",
    ],
  );
});

test("daemon boundary guard allows provider-auth/global-id fixtures", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/global_id_routing_http.rs",
    contents: `
      async fn helper() {
        let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
        let server = fixture.spawn_server().await;
        let daemon = &fixture.daemon;
        let data_root = fixture.data_dir.path();
        daemon.seed_global_id_routing_workspace_session_for_test(seed).await?;
        assert!(!data_root.as_os_str().is_empty());
        assert!(!server.base_url.is_empty());
      }
    `,
    patterns: PROVIDER_AUTH_GLOBAL_ID_FIXTURE_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes provider-auth/global-id fixture roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/codex_host_import_api.rs",
    "core/crates/ctx-http/tests/codex_login_callback_api.rs",
    "core/crates/ctx-http/tests/global_id_routing_http.rs",
  ]) {
    assert.deepEqual(
      providerAuthGlobalIdFixturePatternsForPath(filePath),
      PROVIDER_AUTH_GLOBAL_ID_FIXTURE_PATTERNS,
    );
  }
  assert.deepEqual(
    providerAuthGlobalIdFixturePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects update-route raw daemon fixture setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/common/updates_failure_safety.rs",
    contents: `
      use crate::common::{build_daemon as make_daemon, router_for_daemon as route_for};
      use crate::common::{
        setup_store as open_stores,
      };

      async fn helper(data_root: &std::path::Path) {
        let stores = open_stores(data_root).await;
        let daemon = make_daemon(data_root.to_path_buf(), stores, common::fake_providers(), "http://127.0.0.1:0");
        let _router = route_for(&daemon);
      }
    `,
    patterns: UPDATE_ROUTE_FIXTURE_PATTERNS,
  });

  const names = new Set(violations.map((violation) => violation.name));
  assert(names.has("direct update-route setup store manager helper"));
  assert(names.has("direct update-route setup daemon construction helper"));
  assert(names.has("direct update-route daemon router composition"));
});

test("daemon boundary guard allows update-route bound harness setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/common/updates_failure_safety.rs",
    contents: `
      pub struct UpdateTestApp {
        app: axum::Router,
        _fixture: common::DataRootFakeDaemonFixture,
      }

      impl UpdateTestApp {
        pub fn app(&self) -> axum::Router {
          self.app.clone()
        }
      }

      pub async fn test_app_router(data_root: &std::path::Path) -> UpdateTestApp {
        let fixture = common::fake_daemon_fixture_for_data_root(data_root, "http://127.0.0.1:0").await;
        let app = fixture.router();
        UpdateTestApp { app, _fixture: fixture }
      }
    `,
    patterns: UPDATE_ROUTE_FIXTURE_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes update-route fixture roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/common/updates_failure_safety.rs",
    "core/crates/ctx-http/tests/updates_appimage_apply_safety.rs",
    "core/crates/ctx-http/tests/updates_failure_safety_checksum_mismatch.rs",
    "core/crates/ctx-http/tests/updates_failure_safety_interrupted_transfer.rs",
    "core/crates/ctx-http/tests/updates_failure_safety_manifest_parse.rs",
    "core/crates/ctx-http/tests/updates_failure_safety_manifest_signature.rs",
    "core/crates/ctx-http/tests/updates_failure_safety_missing_artifact.rs",
  ]) {
    assert.deepEqual(
      updateRouteFixturePatternsForPath(filePath),
      UPDATE_ROUTE_FIXTURE_PATTERNS,
    );
  }
  assert.deepEqual(
    updateRouteFixturePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects subscription accounts direct daemon router composition", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/subscription_accounts_api.rs",
    contents: `
      async fn helper(daemon: &TestDaemon, app: axum::Router) {
        let _daemon_backed = common::spawn_http_server(common::router_for_daemon(daemon)).await;
        let _oauth_stub = common::spawn_http_server(app).await;
      }
    `,
    patterns: SUBSCRIPTION_ACCOUNTS_API_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["direct subscription accounts daemon router composition"],
  );
});

test("daemon boundary guard scopes subscription accounts router composition guard", () => {
  assert.deepEqual(
    subscriptionAccountsApiStorePatternsForPath(
      "core/crates/ctx-http/tests/subscription_accounts_api.rs",
    ),
    SUBSCRIPTION_ACCOUNTS_API_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    subscriptionAccountsApiStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct session model store and cache access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/session_model_api.rs",
    contents: `
      use ctx_provider_runtime::CachedProviderOptions;
      async fn helper(daemon: &TestDaemon, store: &Store) {
        let _app = common::router_for_daemon(daemon);
        daemon.test_with_provider_options_cache(|cache| cache.clear()).await;
        store.create_session_with_reasoning_effort(task_id, workspace_id, worktree_id, env, provider, model, effort, agent, None, None, None).await?;
        store.upsert_workspace_session_index(session_id, workspace_id).await?;
      }
    `,
    patterns: SESSION_MODEL_API_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct session model daemon router composition",
      "direct session model provider options cache mutation",
      "direct session model provider options cache mutation",
      "direct session model store seeding",
      "direct session model store seeding",
    ],
  );
});

test("daemon boundary guard scopes session model store facade root", () => {
  assert.deepEqual(
    sessionModelApiStorePatternsForPath("core/crates/ctx-http/tests/session_model_api.rs"),
    SESSION_MODEL_API_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    sessionModelApiStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct cache rehydration store/projection access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/cache_rehydration.rs",
    contents: `
      async fn helper(daemon: &TestDaemon, store: &Store) {
        daemon.global_store().list_workspaces().await?;
        daemon.store_for_session(session_id).await?;
        store.create_workspace(name, root, VcsKind::Git).await?;
        store.insert_session_turn(turn).await?;
        store.append_session_event(session_id, None, None, SessionEventType::Notice, json!({})).await?;
        store.get_session_head_snapshot(session_id, 60, true).await?;
        store.get_session_projection_rev(session_id).await?;
        daemon.publish_session_head_delta(&session, SessionHeadDelta { session_id, ..delta }).await;
      }
    `,
    patterns: CACHE_REHYDRATION_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct cache rehydration store access",
      "direct cache rehydration store access",
      "direct cache rehydration store creation",
      "direct cache rehydration session event write",
      "direct cache rehydration session event write",
      "direct cache rehydration projection read",
      "direct cache rehydration projection read",
      "direct cache rehydration head delta publication",
      "raw cache rehydration event/status model",
    ],
  );
});

test("daemon boundary guard scopes cache rehydration store facade roots", () => {
  assert.deepEqual(
    cacheRehydrationStorePatternsForPath("core/crates/ctx-http/tests/cache_rehydration.rs"),
    CACHE_REHYDRATION_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    cacheRehydrationStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
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

test("daemon boundary guard rejects direct subagent MCP store/probe access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/subagent_mcp_http.rs",
    contents: `
      use ctx_store::Store;
      use ctx_store::{Store as RawStore};
      async fn helper(daemon: &TestDaemon, stores: &StoreManager, store: &Store) {
        let _stores = StoreManager::open(data_dir.path()).await?;
        common::setup_store(data_dir.path()).await;
        common::build_daemon(data_root, stores.clone(), providers, base);
        common::router_for_daemon(&daemon);
        daemon.global_store().list_workspaces().await?;
        daemon.stores().global().await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.store_for_session(session_id).await?;
        daemon.store_for_task(task_id).await?;
        store.create_session(task_id, workspace_id, worktree_id, env, provider, model, role, None, None, None).await?;
        store.update_session_title(session_id, "label".into()).await?;
        store.get_subagent_session_by_label(parent_id, "child").await?;
        store.list_session_turns_page_by_seq(session_id, None, Some(1)).await?;
        store.get_message(message_id).await?;
        store.list_session_events_for_turn(session_id, turn_id, false).await?;
        store.is_archived_subagent_session(session_id).await?;
        store.count_active_subagent_sessions(parent_id).await?;
        store.insert_session_turn(turn).await?;
        store.upsert_session_turn_tool(tool).await?;
        store.upsert_sandbox_binding(binding).await?;
        store.get_worktree(worktree_id).await?;
        store.get_sandbox_binding(worktree_id).await?;
        ctx_workspace_config::update_worktree_bootstrap_config(&store, update).await?;
        daemon.publish_session_head_delta(&session, SessionHeadDelta { session_id, ..delta }, true).await;
      }
    `,
    patterns: SUBAGENT_MCP_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct subagent MCP raw Store type",
      "direct subagent MCP raw Store type",
      "direct subagent MCP StoreManager access",
      "direct subagent MCP StoreManager access",
      "direct subagent MCP common setup helper",
      "direct subagent MCP common setup helper",
      "direct subagent MCP common setup helper",
      "direct subagent MCP daemon store access",
      "direct subagent MCP daemon store access",
      "direct subagent MCP daemon store access",
      "direct subagent MCP daemon store access",
      "direct subagent MCP daemon store access",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw session seed/probe",
      "direct subagent MCP raw turn/tool write",
      "direct subagent MCP raw turn/tool write",
      "direct subagent MCP raw sandbox/worktree probe",
      "direct subagent MCP raw sandbox/worktree probe",
      "direct subagent MCP raw sandbox/worktree probe",
      "direct subagent MCP bootstrap config write",
      "direct subagent MCP head delta publication",
    ],
  );
});

test("daemon boundary guard scopes subagent MCP store facade roots", () => {
  assert.deepEqual(
    subagentMcpStorePatternsForPath("core/crates/ctx-http/tests/subagent_mcp_http.rs"),
    SUBAGENT_MCP_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    subagentMcpStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct replay properties store/projection access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/replay_properties.rs",
    contents: `
      use ctx_store::Store;
      use ctx_store::{Store as RawStore};
      async fn helper(daemon: &TestDaemon, stores: &StoreManager, store: &Store) {
        let _stores = StoreManager::open(data_dir.path()).await?;
        common::setup_store(data_dir.path()).await;
        common::build_daemon(data_root, stores.clone(), providers, base);
        common::router_for_daemon(&daemon);
        daemon.global_store().list_workspaces().await?;
        daemon.stores().global().await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.store_for_session(session_id).await?;
        daemon.store_for_task(task_id).await?;
        store.insert_session_turn(SessionTurn { ..turn }).await?;
        store.insert_message(message).await?;
        store.append_session_event(session_id, None, None, SessionEventType::Notice, payload).await?;
        store.update_session_turn_status(session_id, turn_id, SessionTurnStatus::Completed, None, None, now).await?;
        store.upsert_session_turn_tool(SessionTurnTool { ..tool }).await?;
        daemon.publish_replay_fixture_event_for_test(event).await;
        daemon.refresh_replay_projection_fixture_for_test(workspace_id, session_id).await?;
        daemon.remove_replay_session_head_for_test(session_id).await;
        let _role = MessageRole::Assistant;
        let _delivery = MessageDelivery::Immediate;
      }
    `,
    patterns: REPLAY_PROPERTIES_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct replay properties raw Store type",
      "direct replay properties raw Store type",
      "direct replay properties StoreManager access",
      "direct replay properties StoreManager access",
      "direct replay properties common setup helper",
      "direct replay properties common setup helper",
      "direct replay properties common setup helper",
      "direct replay properties daemon store access",
      "direct replay properties daemon store access",
      "direct replay properties daemon store access",
      "direct replay properties daemon store access",
      "direct replay properties daemon store access",
      "direct replay properties raw row mutation",
      "direct replay properties raw row mutation",
      "direct replay properties raw row mutation",
      "direct replay properties raw row mutation",
      "direct replay properties raw row mutation",
      "direct replay properties raw replay projection helper",
      "direct replay properties raw replay projection helper",
      "direct replay properties raw replay projection helper",
      "direct replay properties raw projection seed model",
      "direct replay properties raw projection seed model",
      "direct replay properties raw projection seed model",
      "direct replay properties raw projection seed model",
      "direct replay properties raw projection seed model",
      "direct replay properties raw projection seed model",
    ],
  );
});

test("daemon boundary guard scopes replay properties store facade roots", () => {
  assert.deepEqual(
    replayPropertiesStorePatternsForPath("core/crates/ctx-http/tests/replay_properties.rs"),
    REPLAY_PROPERTIES_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    replayPropertiesStorePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct session fixture store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/session_artifacts/root_paths.rs",
    contents: `
      use ctx_store::Store;
      async fn helper(daemon: &TestDaemon, store: &Store) {
        let stores = StoreManager::open(data_dir.path()).await?;
        daemon.global_store().list_workspaces().await?;
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.task_session_creation_lock(task_id).await;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        test_daemon_with_providers(data_dir, stores, providers, None);
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
      "legacy session fixture provider daemon construction",
      "raw session fixture ctx_store Store",
      "raw session fixture ctx_store Store",
      "raw session fixture StoreManager",
    ],
  );
});

test("daemon boundary guard scopes session fixture store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/lib_tests/daemon_smoke.rs",
    "core/crates/ctx-http/src/lib_tests/daemon_smoke/golden_path.rs",
    "core/crates/ctx-http/src/lib_tests/daemon_smoke/streaming/fixture.rs",
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
  assert.deepEqual(
    sessionFixtureStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct small-boundary store and handle access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/update_boundaries.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        daemon
          .handle()
          .sessions();
        daemon
          .handle()
          .workspaces();
        let _raw: Store;
      }
    `,
    patterns: SMALL_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct small-boundary global store access",
      "direct small-boundary session store access",
      "direct small-boundary workspace store access",
      "direct small-boundary uncached workspace store access",
      "direct small-boundary task store access",
      "direct small-boundary StoreManager access",
      "direct small-boundary StoreManager global access",
      "direct small-boundary StoreManager workspace access",
      "direct sessions handle access in migrated test",
      "direct workspaces handle access in migrated test",
      "raw small-boundary ctx_store Store",
      "raw small-boundary ctx_store Store",
    ],
  );
});

test("daemon boundary guard rejects providerless lib-route direct StoreManager setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/cors.rs",
    contents: `
      use ctx_store::StoreManager;
      async fn fixture() {
        let _stores = StoreManager::open(data_dir.path()).await.unwrap();
      }
    `,
    patterns: smallBoundaryStorePatternsForPath("core/crates/ctx-http/src/lib_tests/cors.rs"),
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw providerless lib-route StoreManager",
      "raw providerless lib-route StoreManager",
    ],
  );
});

test("daemon boundary guard rejects execution-launch direct store setup", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/execution_launch/host_mode.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores();
        stores.global();
        stores.workspace(workspace_id);
        test_daemon_with_providers(data_dir, stores, providers, None);
      }
    `,
    patterns: executionLaunchStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/execution_launch/host_mode.rs",
    ),
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct execution-launch global store access",
      "direct execution-launch session store access",
      "direct execution-launch workspace store access",
      "direct execution-launch uncached workspace store access",
      "direct execution-launch task store access",
      "direct execution-launch StoreManager access",
      "direct execution-launch StoreManager global access",
      "direct execution-launch StoreManager workspace access",
      "legacy execution-launch provider daemon construction",
      "raw execution-launch ctx_store Store",
      "raw execution-launch StoreManager",
      "raw execution-launch StoreManager",
    ],
  );
});

test("daemon boundary guard scopes small-boundary store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/lib_tests/cors.rs",
    "core/crates/ctx-http/src/lib_tests/health_diagnostics/auth.rs",
    "core/crates/ctx-http/src/lib_tests/health_diagnostics/managed_config.rs",
    "core/crates/ctx-http/src/lib_tests/health_diagnostics/storage.rs",
    "core/crates/ctx-http/src/lib_tests/org_policy_routes.rs",
    "core/crates/ctx-http/src/lib_tests/run_archive_routes.rs",
    "core/crates/ctx-http/src/lib_tests/telemetry_export_boundaries.rs",
    "core/crates/ctx-http/src/lib_tests/update_boundaries.rs",
    "core/crates/ctx-http/src/lib_tests/workspace_active_routes.rs",
  ]) {
    assert.deepEqual(
      smallBoundaryStorePatternsForPath(filePath),
      [
        ...SMALL_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
        ...PROVIDERLESS_LIB_ROUTE_TEST_STORE_ACCESS_PATTERNS,
      ],
    );
  }

  for (const filePath of [
    "core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs",
    "core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs",
    "core/crates/ctx-http/src/lib_tests/web_session_routes/fixtures.rs",
  ]) {
    assert.deepEqual(
      smallBoundaryStorePatternsForPath(filePath),
      PROVIDERLESS_LIB_ROUTE_TEST_STORE_ACCESS_PATTERNS,
    );
  }

  for (const filePath of [
    "core/crates/ctx-http/src/api/sessions/tests.rs",
    "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
    "core/crates/ctx-http/src/api/workspaces/tests.rs",
  ]) {
    assert.deepEqual(
      smallBoundaryStorePatternsForPath(filePath),
      SMALL_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    smallBoundaryStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/execution_launch/settings_errors.rs",
    ),
    [],
  );
  assert.deepEqual(
    executionLaunchStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/execution_launch/settings_errors.rs",
    ),
    EXECUTION_LAUNCH_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    executionLaunchStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/execution_launch.rs",
    ),
    EXECUTION_LAUNCH_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    executionLaunchStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/cors.rs",
    ),
    [],
  );
  assert.deepEqual(
    smallBoundaryStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/provider_routes.rs",
    ),
    [],
  );
  assert.deepEqual(
    smallBoundaryStorePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct global-id-routing store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/global_id_routing_http.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        let _raw: Store;
      }
    `,
    patterns: GLOBAL_ID_ROUTING_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct global-id-routing global store access",
      "direct global-id-routing session store access",
      "direct global-id-routing workspace store access",
      "direct global-id-routing uncached workspace store access",
      "direct global-id-routing task store access",
      "direct global-id-routing StoreManager access",
      "direct global-id-routing StoreManager global access",
      "direct global-id-routing StoreManager workspace access",
      "raw global-id-routing ctx_store Store",
      "raw global-id-routing ctx_store Store",
      "raw global-id-routing StoreManager",
      "raw global-id-routing StoreManager",
    ],
  );
});

test("daemon boundary guard scopes global-id-routing store facade root", () => {
  assert.deepEqual(
    globalIdRoutingStorePatternsForPath(
      "core/crates/ctx-http/tests/global_id_routing_http.rs",
    ),
    GLOBAL_ID_ROUTING_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    globalIdRoutingStorePatternsForPath(
      "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct terminal-workspace-stream store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        let _raw: Store;
      }
    `,
    patterns: TERMINAL_WORKSPACE_STREAM_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct terminal-workspace-stream global store access",
      "direct terminal-workspace-stream session store access",
      "direct terminal-workspace-stream workspace store access",
      "direct terminal-workspace-stream uncached workspace store access",
      "direct terminal-workspace-stream task store access",
      "direct terminal-workspace-stream StoreManager access",
      "direct terminal-workspace-stream StoreManager global access",
      "direct terminal-workspace-stream StoreManager workspace access",
      "raw terminal-workspace-stream ctx_store Store",
      "raw terminal-workspace-stream ctx_store Store",
      "raw terminal-workspace-stream StoreManager",
      "raw terminal-workspace-stream StoreManager",
    ],
  );
});

test("daemon boundary guard scopes terminal-workspace-stream store facade root", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
    "core/crates/ctx-http/tests/terminal_ws_reconnect.rs",
  ]) {
    assert.deepEqual(
      terminalWorkspaceStreamStorePatternsForPath(filePath),
      TERMINAL_WORKSPACE_STREAM_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    terminalWorkspaceStreamStorePatternsForPath(
      "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct workspace-runtime-settings store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        let _raw: Store;
      }
    `,
    patterns: WORKSPACE_RUNTIME_SETTINGS_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct workspace-runtime-settings global store access",
      "direct workspace-runtime-settings session store access",
      "direct workspace-runtime-settings workspace store access",
      "direct workspace-runtime-settings uncached workspace store access",
      "direct workspace-runtime-settings task store access",
      "direct workspace-runtime-settings StoreManager access",
      "direct workspace-runtime-settings StoreManager global access",
      "direct workspace-runtime-settings StoreManager workspace access",
      "raw workspace-runtime-settings ctx_store Store",
      "raw workspace-runtime-settings ctx_store Store",
      "raw workspace-runtime-settings StoreManager",
      "raw workspace-runtime-settings StoreManager",
    ],
  );
});

test("daemon boundary guard allows common setup-store in workspace-runtime-settings", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
    contents: `
      async fn fixture() {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
      }
    `,
    patterns: WORKSPACE_RUNTIME_SETTINGS_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes workspace-runtime-settings store facade root", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/session_diff_unavailable.rs",
    "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
  ]) {
    assert.deepEqual(
      workspaceRuntimeSettingsStorePatternsForPath(filePath),
      WORKSPACE_RUNTIME_SETTINGS_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    workspaceRuntimeSettingsStorePatternsForPath(
      "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct workspace-merge-queue-config store and handle access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    contents: `
      use ctx_daemon::daemon::merge_queue;
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        daemon
          .handle()
          .workspaces();
        let handle = daemon.handle();
        handle.workspaces();
        merge_queue::activate_workspace_merge_queue(&state, workspace_id).await;
        manager.global().await?;
        manager.workspace(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace(workspace_id).await?;
        let _raw: Store;
      }
    `,
    patterns: WORKSPACE_MERGE_QUEUE_CONFIG_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct workspace-merge-queue-config global store access",
      "direct workspace-merge-queue-config session store access",
      "direct workspace-merge-queue-config workspace store access",
      "direct workspace-merge-queue-config uncached workspace store access",
      "direct workspace-merge-queue-config task store access",
      "direct workspace-merge-queue-config StoreManager access",
      "direct workspace-merge-queue-config StoreManager global access",
      "direct workspace-merge-queue-config StoreManager global access",
      "direct workspace-merge-queue-config StoreManager global access",
      "direct workspace-merge-queue-config StoreManager workspace access",
      "direct workspace-merge-queue-config StoreManager workspace access",
      "direct workspace-merge-queue-config StoreManager workspace access",
      "direct workspace-merge-queue-config workspaces handle access",
      "direct workspace-merge-queue-config workspaces handle access",
      "direct workspace-merge-queue-config workspaces handle access",
      "direct workspace-merge-queue-config daemon merge-queue module access",
      "direct workspace-merge-queue-config daemon merge-queue module access",
      "raw workspace-merge-queue-config ctx_store Store",
      "raw workspace-merge-queue-config ctx_store Store",
      "raw workspace-merge-queue-config StoreManager",
      "raw workspace-merge-queue-config StoreManager",
    ],
  );
});

test("daemon boundary guard allows common setup-store in workspace-merge-queue-config", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    contents: `
      async fn fixture() {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
      }
    `,
    patterns: WORKSPACE_MERGE_QUEUE_CONFIG_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes workspace-merge-queue-config store facade root", () => {
  assert.deepEqual(
    workspaceMergeQueueConfigStorePatternsForPath(
      "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    ),
    WORKSPACE_MERGE_QUEUE_CONFIG_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    workspaceMergeQueueConfigStorePatternsForPath(
      "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct jj-merge-queue-basics store and worktree access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    contents: `
      use ctx_daemon::daemon::merge_queue;
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        let store = daemon.store_for_task(task.id).await.unwrap();
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        stores.workspace_uncached(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace(workspace_id).await?;
        manager
          .workspace_uncached(workspace_id).await?;
        daemon.handle().workspaces();
        daemon.handle().sessions();
        let handle = daemon.handle();
        handle.workspaces();
        handle.sessions();
        merge_queue::activate_workspace_merge_queue(&state, workspace_id).await;
        store.get_worktree(session.worktree_id).await?;
        daemon.load_worktree_for_test(session.worktree_id).await?;
        let _raw: Store;
      }
    `,
    patterns: JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct jj-merge-queue-basics global store access",
      "direct jj-merge-queue-basics session store access",
      "direct jj-merge-queue-basics workspace store access",
      "direct jj-merge-queue-basics uncached workspace store access",
      "direct jj-merge-queue-basics task store access",
      "direct jj-merge-queue-basics StoreManager access",
      "direct jj-merge-queue-basics StoreManager global access",
      "direct jj-merge-queue-basics StoreManager global access",
      "direct jj-merge-queue-basics StoreManager workspace access",
      "direct jj-merge-queue-basics StoreManager workspace access",
      "direct jj-merge-queue-basics workspaces handle access",
      "direct jj-merge-queue-basics workspaces handle access",
      "direct jj-merge-queue-basics workspaces handle access",
      "direct jj-merge-queue-basics sessions handle access",
      "direct jj-merge-queue-basics sessions handle access",
      "direct jj-merge-queue-basics sessions handle access",
      "direct jj-merge-queue-basics daemon merge-queue module access",
      "direct jj-merge-queue-basics daemon merge-queue module access",
      "direct jj-merge-queue-basics worktree row load",
      "direct jj-merge-queue-basics worktree row load",
      "raw jj-merge-queue-basics ctx_store Store",
      "raw jj-merge-queue-basics ctx_store Store",
      "raw jj-merge-queue-basics StoreManager",
      "raw jj-merge-queue-basics StoreManager",
    ],
  );
});

test("daemon boundary guard allows common setup-store in jj-merge-queue-basics", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    contents: `
      async fn fixture() {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
      }
    `,
    patterns: JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes jj-merge-queue-basics store facade root", () => {
  assert.deepEqual(
    jjMergeQueueBasicsStorePatternsForPath(
      "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    ),
    JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    jjMergeQueueBasicsStorePatternsForPath(
      "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct merge-queue-isolation store/config access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/merge_queue_isolation.rs",
    contents: `
      use ctx_daemon::daemon::merge_queue;
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager, store: Store) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.store_for_worktree(worktree_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        stores.workspace_uncached(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace(workspace_id).await?;
        manager
          .workspace_uncached(workspace_id).await?;
        daemon.handle().workspaces();
        daemon.handle().sessions();
        let handle = daemon.handle();
        handle.workspaces();
        handle.sessions();
        ctx_workspace_config::update_merge_queue_config(&store, update).await?;
        let _update: MergeQueueConfigUpdate;
        let _sync = MergeQueueCanonicalSync::Never;
        merge_queue::activate_workspace_merge_queue(&state, workspace_id).await;
        store.create_worktree(workspace_id, root, head, branch).await?;
        store.insert_worktree(worktree).await?;
        store.get_worktree(worktree_id).await?;
        daemon.load_worktree_for_test(worktree_id).await?;
        daemon.global_store().upsert_workspace_worktree_index(worktree_id, workspace_id).await?;
        store.get_merge_queue_entry(entry_id).await?;
        store.list_merge_queue_entries(workspace_id, Some(1)).await?;
        store.get_latest_merge_queue_run(entry_id).await?;
        store.create_merge_queue_entry(&entry).await?;
        store.create_merge_queue_run(&run).await?;
        let _raw: Store;
      }
    `,
    patterns: MERGE_QUEUE_ISOLATION_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct merge-queue-isolation global store access",
      "direct merge-queue-isolation global store access",
      "direct merge-queue-isolation session store access",
      "direct merge-queue-isolation workspace store access",
      "direct merge-queue-isolation uncached workspace store access",
      "direct merge-queue-isolation task store access",
      "direct merge-queue-isolation worktree store access",
      "direct merge-queue-isolation StoreManager access",
      "direct merge-queue-isolation StoreManager global access",
      "direct merge-queue-isolation StoreManager global access",
      "direct merge-queue-isolation StoreManager workspace access",
      "direct merge-queue-isolation StoreManager workspace access",
      "direct merge-queue-isolation StoreManager workspace access",
      "direct merge-queue-isolation StoreManager workspace access",
      "direct merge-queue-isolation workspaces handle access",
      "direct merge-queue-isolation workspaces handle access",
      "direct merge-queue-isolation workspaces handle access",
      "direct merge-queue-isolation sessions handle access",
      "direct merge-queue-isolation sessions handle access",
      "direct merge-queue-isolation sessions handle access",
      "direct merge-queue-isolation workspace config write",
      "direct merge-queue-isolation workspace config write",
      "direct merge-queue-isolation workspace config write",
      "direct merge-queue-isolation daemon merge-queue module access",
      "direct merge-queue-isolation daemon merge-queue module access",
      "direct merge-queue-isolation worktree row access",
      "direct merge-queue-isolation worktree row access",
      "direct merge-queue-isolation worktree row access",
      "direct merge-queue-isolation worktree row access",
      "direct merge-queue-isolation worktree row access",
      "direct merge-queue-isolation entry or run row access",
      "direct merge-queue-isolation entry or run row access",
      "direct merge-queue-isolation entry or run row access",
      "direct merge-queue-isolation entry or run row access",
      "direct merge-queue-isolation entry or run row access",
      "raw merge-queue-isolation ctx_store Store",
      "raw merge-queue-isolation ctx_store Store",
      "raw merge-queue-isolation ctx_store Store",
      "raw merge-queue-isolation StoreManager",
      "raw merge-queue-isolation StoreManager",
    ],
  );
});

test("daemon boundary guard scopes merge-queue-isolation store facade root", () => {
  assert.deepEqual(
    mergeQueueIsolationStorePatternsForPath("core/crates/ctx-http/tests/merge_queue_isolation.rs"),
    MERGE_QUEUE_ISOLATION_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    mergeQueueIsolationStorePatternsForPath(
      "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct provider-worker-reaping store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
    contents: `
      use ctx_core::models::SessionEventType;
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.store_for_worktree(worktree_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace_uncached(workspace_id).await?;
        store.list_session_events(session_id).await?;
        store.get_session(session_id).await?;
        daemon.handle().sessions();
        daemon.handle().workspaces();
        daemon.handle().tasks();
        let handle = daemon.handle();
        handle.sessions();
        handle.workspaces();
        handle.tasks();
        let _raw: Store;
      }
    `,
    patterns: PROVIDER_WORKER_REAPING_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct provider-worker-reaping global store access",
      "direct provider-worker-reaping session store access",
      "direct provider-worker-reaping workspace store access",
      "direct provider-worker-reaping uncached workspace store access",
      "direct provider-worker-reaping task store access",
      "direct provider-worker-reaping worktree store access",
      "direct provider-worker-reaping StoreManager access",
      "direct provider-worker-reaping StoreManager global access",
      "direct provider-worker-reaping StoreManager global access",
      "direct provider-worker-reaping StoreManager workspace access",
      "direct provider-worker-reaping StoreManager workspace access",
      "direct provider-worker-reaping session event query",
      "direct provider-worker-reaping session row query",
      "direct provider-worker-reaping sessions handle access",
      "direct provider-worker-reaping sessions handle access",
      "direct provider-worker-reaping sessions handle access",
      "direct provider-worker-reaping workspaces handle access",
      "direct provider-worker-reaping workspaces handle access",
      "direct provider-worker-reaping workspaces handle access",
      "direct provider-worker-reaping tasks handle access",
      "direct provider-worker-reaping tasks handle access",
      "direct provider-worker-reaping tasks handle access",
      "direct provider-worker-reaping SessionEventType",
      "raw provider-worker-reaping ctx_store Store",
      "raw provider-worker-reaping ctx_store Store",
      "raw provider-worker-reaping StoreManager",
      "raw provider-worker-reaping StoreManager",
    ],
  );
});

test("daemon boundary guard allows common setup-store in provider-worker-reaping", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
    contents: `
      async fn fixture() {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
      }
    `,
    patterns: PROVIDER_WORKER_REAPING_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes provider-worker-reaping store facade root", () => {
  assert.deepEqual(
    providerWorkerReapingStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
    ),
    PROVIDER_WORKER_REAPING_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    providerWorkerReapingStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct provider-scenarios offline store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, providers, "http://127.0.0.1:0");
        let app = common::router_for_daemon(&daemon);
        let daemon = TestDaemon::new(data_root, stores, providers, base_url, None);
        daemon.store_for_session(session_id).await?;
        daemon.stores().global().await?;
        store.list_session_events(session_id).await?;
        store.list_session_turns_page_by_seq(session_id, None, Some(10)).await?;
        store.list_messages_for_session(session_id).await?;
        let status = SessionTurnStatus::Completed;
        let _raw: Store;
      }
    `,
    patterns: PROVIDER_SCENARIOS_OFFLINE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct provider-scenarios offline StoreManager access",
      "direct provider-scenarios offline StoreManager access",
      "raw provider-scenarios offline ctx_store Store",
      "direct provider-scenarios offline store manager helper",
      "direct provider-scenarios offline store manager helper",
      "direct provider-scenarios offline daemon construction helper",
      "direct provider-scenarios offline daemon construction helper",
      "direct provider-scenarios offline router composition",
      "direct provider-scenarios offline router composition",
      "direct provider-scenarios offline TestDaemon construction",
      "direct provider-scenarios offline daemon store access",
      "direct provider-scenarios offline daemon store access",
      "direct provider-scenarios offline session event query",
      "direct provider-scenarios offline session turn query",
      "direct provider-scenarios offline session message query",
      "direct provider-scenarios offline turn-status polling",
    ],
  );
});

test("daemon boundary guard scopes provider-scenarios offline store facade root", () => {
  assert.deepEqual(
    providerScenariosOfflineStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
    ),
    PROVIDER_SCENARIOS_OFFLINE_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    providerScenariosOfflineStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct harness-container sandbox store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/harness_container_sandbox_e2e.rs",
    contents: `
      use ctx_settings_service::{load_settings, save_settings};
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        let settings = load_settings(&store).await?;
        save_settings(&store, &settings).await?;
        let store = Store::open_sqlite(&db_path, None).await?;
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, providers, "http://127.0.0.1:0");
        let app = common::router_for_daemon(&daemon);
        let daemon = TestDaemon::new(data_root, stores, providers, base_url, None);
        daemon.global_store().get_workspace(workspace_id).await?;
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.stores().global().await?;
        store.list_session_events(session_id).await?;
        workspace_store.get_worktree(worktree_id).await?;
        daemon.prepare_workspace_harness_for_test(&workspace, &worktree, &execution).await?;
        daemon.workspace_harness_egress_guard_for_test(workspace_id).await?;
        let _event = SessionEventType::Done;
        let _raw: Store;
      }
    `,
    patterns: HARNESS_CONTAINER_SANDBOX_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct harness-container sandbox StoreManager access",
      "direct harness-container sandbox StoreManager access",
      "raw harness-container sandbox ctx_store Store",
      "raw harness-container sandbox ctx_store Store",
      "direct harness-container sandbox settings service",
      "direct harness-container sandbox settings service",
      "direct harness-container sandbox settings service",
      "direct harness-container sandbox common setup helper",
      "direct harness-container sandbox common setup helper",
      "direct harness-container sandbox common setup helper",
      "direct harness-container sandbox common setup helper",
      "direct harness-container sandbox TestDaemon construction",
      "direct harness-container sandbox daemon store access",
      "direct harness-container sandbox daemon store access",
      "direct harness-container sandbox daemon store access",
      "direct harness-container sandbox daemon store access",
      "direct harness-container sandbox raw session event query",
      "direct harness-container sandbox raw session event query",
      "direct harness-container sandbox raw workspace/worktree query",
      "direct harness-container sandbox raw workspace/worktree query",
      "direct harness-container sandbox daemon harness prep",
      "direct harness-container sandbox daemon harness prep",
    ],
  );
});

test("daemon boundary guard scopes harness-container sandbox store facade root", () => {
  assert.deepEqual(
    harnessContainerSandboxStorePatternsForPath(
      "core/crates/ctx-http/tests/harness_container_sandbox_e2e.rs",
    ),
    HARNESS_CONTAINER_SANDBOX_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    harnessContainerSandboxStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct live-provider canary store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/live_provider_canary.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        let stores = StoreManager::open(data_dir.path()).await?;
        let store = Store::open_sqlite(&db_path, None).await?;
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, providers, "http://127.0.0.1:0");
        let app = common::router_for_daemon(&daemon);
        let daemon = TestDaemon::new(data_root, stores, providers, base_url, None);
        daemon.global_store().get_workspace(workspace_id).await?;
        daemon.store_for_session(session_id).await?;
        daemon.stores().global().await?;
        store.list_session_events(session_id).await?;
        let _event = SessionEventType::Done;
        let _messages = assistant_messages_from_events(&events);
        let _raw: Store;
      }
    `,
    patterns: LIVE_PROVIDER_CANARY_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct live-provider canary StoreManager access",
      "direct live-provider canary StoreManager access",
      "direct live-provider canary StoreManager access",
      "raw live-provider canary ctx_store Store",
      "raw live-provider canary ctx_store Store",
      "direct live-provider canary common setup helper",
      "direct live-provider canary common setup helper",
      "direct live-provider canary common setup helper",
      "direct live-provider canary common setup helper",
      "direct live-provider canary TestDaemon construction",
      "direct live-provider canary daemon store access",
      "direct live-provider canary daemon store access",
      "direct live-provider canary daemon store access",
      "direct live-provider canary raw session event query",
      "direct live-provider canary raw session event query",
      "direct live-provider canary event-message extraction helper",
    ],
  );
});

test("daemon boundary guard scopes live-provider canary store facade root", () => {
  assert.deepEqual(
    liveProviderCanaryStorePatternsForPath(
      "core/crates/ctx-http/tests/live_provider_canary.rs",
    ),
    LIVE_PROVIDER_CANARY_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    liveProviderCanaryStorePatternsForPath(
      "core/crates/ctx-http/tests/harness_container_sandbox_e2e.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct worktree-archive store and path access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/worktree_archive_http.rs",
    contents: `
      use ctx_daemon::daemon::workspaces::managed_worktree_root;
      use ctx_fs::worktrees::managed_worktree_path;
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.store_for_worktree(worktree_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace_uncached(workspace_id).await?;
        store.get_task(task_id).await?;
        store.list_sessions_for_task(task_id).await?;
        store.get_worktree(worktree_id).await?;
        daemon.load_worktree_for_test(worktree_id).await?;
        daemon.handle().sessions();
        daemon.handle().workspaces();
        daemon.handle().tasks();
        let handle = daemon.handle();
        handle.sessions();
        handle.workspaces();
        handle.tasks();
        let root = managed_worktree_path(data_dir.path(), workspace_id, worktree_id);
        let root = managed_worktree_root(state, workspace, worktree);
        let _raw: Store;
      }
    `,
    patterns: WORKTREE_ARCHIVE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct worktree-archive global store access",
      "direct worktree-archive session store access",
      "direct worktree-archive workspace store access",
      "direct worktree-archive uncached workspace store access",
      "direct worktree-archive task store access",
      "direct worktree-archive worktree store access",
      "direct worktree-archive StoreManager access",
      "direct worktree-archive StoreManager global access",
      "direct worktree-archive StoreManager global access",
      "direct worktree-archive StoreManager workspace access",
      "direct worktree-archive StoreManager workspace access",
      "direct worktree-archive task row query",
      "direct worktree-archive task-session query",
      "direct worktree-archive worktree row query",
      "direct worktree-archive worktree row query",
      "direct worktree-archive sessions handle access",
      "direct worktree-archive sessions handle access",
      "direct worktree-archive sessions handle access",
      "direct worktree-archive workspaces handle access",
      "direct worktree-archive workspaces handle access",
      "direct worktree-archive workspaces handle access",
      "direct worktree-archive tasks handle access",
      "direct worktree-archive tasks handle access",
      "direct worktree-archive tasks handle access",
      "direct worktree-archive managed worktree path reconstruction",
      "direct worktree-archive managed worktree path reconstruction",
      "direct worktree-archive managed worktree path reconstruction",
      "direct worktree-archive managed worktree path reconstruction",
      "raw worktree-archive ctx_store Store",
      "raw worktree-archive ctx_store Store",
      "raw worktree-archive StoreManager",
      "raw worktree-archive StoreManager",
    ],
  );
});

test("daemon boundary guard allows common setup-store in worktree-archive", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/worktree_archive_http.rs",
    contents: `
      async fn fixture() {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
      }
    `,
    patterns: WORKTREE_ARCHIVE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes worktree-archive store facade root", () => {
  assert.deepEqual(
    worktreeArchiveStorePatternsForPath(
      "core/crates/ctx-http/tests/worktree_archive_http.rs",
    ),
    WORKTREE_ARCHIVE_TEST_STORE_ACCESS_PATTERNS,
  );
  assert.deepEqual(
    worktreeArchiveStorePatternsForPath(
      "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct fault-injection store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/fault_matrix.rs",
    contents: `
      use ctx_core::models::SessionEventType;
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.store_for_worktree(worktree_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace_uncached(workspace_id).await?;
        store.list_sessions_for_task(task_id).await?;
        store.append_session_event(session_id, None, None, SessionEventType::Notice, json!({})).await?;
        store.get_session_head_snapshot(session_id, 10, true).await?;
        daemon.handle().sessions();
        daemon.handle().workspaces();
        daemon.handle().tasks();
        let handle = daemon.handle();
        handle.sessions();
        handle.workspaces();
        handle.tasks();
        let _raw: Store;
      }
    `,
    patterns: FAULT_INJECTION_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct fault-injection global store access",
      "direct fault-injection session store access",
      "direct fault-injection workspace store access",
      "direct fault-injection uncached workspace store access",
      "direct fault-injection task store access",
      "direct fault-injection worktree store access",
      "direct fault-injection StoreManager access",
      "direct fault-injection StoreManager global access",
      "direct fault-injection StoreManager global access",
      "direct fault-injection StoreManager workspace access",
      "direct fault-injection StoreManager workspace access",
      "direct fault-injection task-session query",
      "direct fault-injection session event append",
      "direct fault-injection session head store query",
      "direct fault-injection sessions handle access",
      "direct fault-injection sessions handle access",
      "direct fault-injection sessions handle access",
      "direct fault-injection workspaces handle access",
      "direct fault-injection workspaces handle access",
      "direct fault-injection workspaces handle access",
      "direct fault-injection tasks handle access",
      "direct fault-injection tasks handle access",
      "direct fault-injection tasks handle access",
      "direct fault-injection SessionEventType",
      "direct fault-injection SessionEventType",
      "raw fault-injection ctx_store Store",
      "raw fault-injection ctx_store Store",
      "raw fault-injection StoreManager",
      "raw fault-injection StoreManager",
    ],
  );
});

test("daemon boundary guard allows intentional fault-injection controls", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/hot_endpoints_no_db.rs",
    contents: `
      fn fixture() {
        ctx_store::fault_injection::clear_failpoints();
        ctx_store::fault_injection::set_failpoint("ctx_store.get_session_head_snapshot", 1);
        ctx_http::fault_injection::set_failpoint("ctx_http.send_workspace_active_reset", 1);
      }
    `,
    patterns: FAULT_INJECTION_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes fault-injection store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/fault_matrix.rs",
    "core/crates/ctx-http/tests/hot_endpoints_no_db.rs",
  ]) {
    assert.deepEqual(
      faultInjectionStorePatternsForPath(filePath),
      FAULT_INJECTION_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    faultInjectionStorePatternsForPath(
      "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct stream-runtime store and handle access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        manager
          .global().await?;
        manager
          .workspace(workspace_id).await?;
        daemon.handle().sessions();
        daemon.handle().workspaces();
        let handle = daemon.handle();
        handle.sessions();
        handle.workspaces();
        let _raw: Store;
      }
    `,
    patterns: STREAM_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct stream-runtime global store access",
      "direct stream-runtime session store access",
      "direct stream-runtime workspace store access",
      "direct stream-runtime uncached workspace store access",
      "direct stream-runtime task store access",
      "direct stream-runtime StoreManager access",
      "direct stream-runtime StoreManager global access",
      "direct stream-runtime StoreManager global access",
      "direct stream-runtime StoreManager workspace access",
      "direct stream-runtime StoreManager workspace access",
      "direct stream-runtime sessions handle access",
      "direct stream-runtime sessions handle access",
      "direct stream-runtime sessions handle access",
      "direct stream-runtime workspaces handle access",
      "direct stream-runtime workspaces handle access",
      "direct stream-runtime workspaces handle access",
      "raw stream-runtime ctx_store Store",
      "raw stream-runtime ctx_store Store",
      "raw stream-runtime StoreManager",
      "raw stream-runtime StoreManager",
    ],
  );
});

test("daemon boundary guard allows common setup-store in stream-runtime", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
    contents: `
      async fn fixture() {
        let stores = common::setup_store(data_dir.path()).await;
        let daemon = common::build_daemon(data_dir.path(), stores, common::fake_providers(), "http://127.0.0.1:0");
      }
    `,
    patterns: STREAM_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes stream-runtime store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
    "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
    "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
    "core/crates/ctx-http/tests/workspace_stream_stress_active_heads_lag.rs",
  ]) {
    assert.deepEqual(
      streamRuntimeStorePatternsForPath(filePath),
      STREAM_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    streamRuntimeStorePatternsForPath(
      "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects direct scheduler-runtime store and handle access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        daemon
          .handle()
          .sessions();
        let _raw: Store;
      }
    `,
    patterns: SCHEDULER_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct scheduler-runtime global store access",
      "direct scheduler-runtime session store access",
      "direct scheduler-runtime workspace store access",
      "direct scheduler-runtime uncached workspace store access",
      "direct scheduler-runtime task store access",
      "direct scheduler-runtime StoreManager access",
      "direct scheduler-runtime StoreManager global access",
      "direct scheduler-runtime StoreManager workspace access",
      "direct scheduler-runtime sessions handle access",
      "raw scheduler-runtime ctx_store Store",
      "raw scheduler-runtime ctx_store Store",
    ],
  );
});

test("daemon boundary guard scopes scheduler-runtime store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
    "core/crates/ctx-http/tests/assistant_message_persistence_faults.rs",
    "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
    "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
  ]) {
    assert.deepEqual(
      schedulerRuntimeStorePatternsForPath(filePath),
      SCHEDULER_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(
    schedulerRuntimeStorePatternsForPath("core/crates/ctx-http/tests/noisy_output_backpressure.rs"),
    [],
  );
});

test("daemon boundary guard rejects direct task-lifecycle store access", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/tasks/lifecycle_tests/fixtures.rs",
    contents: `
      use ctx_settings_service::save_settings;
      use ctx_settings_service::{
        load_settings,
        save_settings as persist_settings,
      };
      use ctx_store::{Store, StoreManager};
      async fn fixture(daemon: TestDaemon, stores: StoreManager) {
        daemon.global_store();
        daemon.store_for_session(session_id).await?;
        daemon.store_for_workspace(workspace_id).await?;
        daemon.uncached_store_for_workspace(workspace_id).await?;
        daemon.store_for_task(task_id).await?;
        daemon.stores().global().await?;
        stores.global().await?;
        stores.workspace(workspace_id).await?;
        ctx_settings_service::save_settings(daemon.global_store(), &settings).await?;
        save_settings(daemon.global_store(), &settings).await?;
        persist_settings(daemon.global_store(), &settings).await?;
        let daemon = TestDaemon::new(data_dir, stores, providers, "http://127.0.0.1:0".into(), None);
        let daemon = TestDaemon::new_for_test(data_dir, "http://127.0.0.1:0".into()).await?;
        daemon
          .handle()
          .workspaces();
        let _raw: Store;
      }
    `,
    patterns: TASK_LIFECYCLE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct task-lifecycle global store access",
      "direct task-lifecycle global store access",
      "direct task-lifecycle global store access",
      "direct task-lifecycle global store access",
      "direct task-lifecycle session store access",
      "direct task-lifecycle workspace store access",
      "direct task-lifecycle uncached workspace store access",
      "direct task-lifecycle task store access",
      "direct task-lifecycle StoreManager access",
      "direct task-lifecycle StoreManager global access",
      "direct task-lifecycle StoreManager workspace access",
      "direct task-lifecycle workspaces handle access",
      "raw task-lifecycle ctx_store Store",
      "raw task-lifecycle ctx_store Store",
      "raw task-lifecycle StoreManager",
      "raw task-lifecycle StoreManager",
      "direct task-lifecycle raw TestDaemon construction",
      "direct task-lifecycle raw TestDaemon construction",
      "direct task-lifecycle settings persistence",
      "direct task-lifecycle settings persistence",
      "direct task-lifecycle settings persistence",
      "direct task-lifecycle settings persistence",
    ],
  );
});

test("daemon boundary guard rejects storage-admission raw daemon/router helpers", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs",
    contents: `
      async fn helper(state: &TestDaemon) {
        let daemon = TestDaemon::new_for_test(data_root, "http://127.0.0.1:4311".to_string()).await?;
        let daemon = TestDaemon::new_with_providers_for_test(data_root, providers, "http://127.0.0.1:4311".to_string(), None).await?;
        state.save_execution_settings_for_test(execution).await?;
        test_router(state);
        crate::api::router(crate::api::RouteHandles::from_daemon_handle(state.handle()))
      }
    `,
    patterns: STORAGE_ADMISSION_FIXTURE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct storage-admission raw TestDaemon construction",
      "direct storage-admission raw TestDaemon construction",
      "direct storage-admission router helper",
      "direct storage-admission router composition",
    ],
  );
});

test("daemon boundary guard allows task-lifecycle data-root daemon fixture", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/tasks/lifecycle_tests/fixtures.rs",
    contents: `
      async fn helper(data_root: &StdPath) {
        let fixture = crate::test_support::DataRootTestDaemonFixture::with_providers(
          data_root,
          HashMap::new(),
          "http://127.0.0.1:4310",
        ).await;
        fixture.daemon().seed_task_lifecycle_task_for_test(workspace, "task").await?;
      }
    `,
    patterns: TASK_LIFECYCLE_TEST_STORE_ACCESS_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard allows storage-admission data-root daemon fixture", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs",
    contents: `
      async fn helper(data_root: &Path) {
        let fixture = crate::test_support::DataRootTestDaemonFixture::new(data_root, "http://127.0.0.1:4311").await;
        fixture.daemon().save_execution_settings_for_test(execution).await?;
        let app = fixture.router();
      }
    `,
    patterns: STORAGE_ADMISSION_FIXTURE_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard rejects lib-test raw daemon/router helpers", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    contents: `
      use ctx_daemon::test_support::TestDaemon as RawDaemon;
      use ctx_daemon::test_support::{Other, TestDaemon as GroupedRawDaemon};
      async fn helper(state: &TestDaemon) {
        let daemon = TestDaemon::new_for_test(data_root, base_url).await?;
        let daemon = RawDaemon::new_with_providers_for_test(data_root, providers, base_url, None).await?;
        let daemon = GroupedRawDaemon::new_for_test(data_root, base_url).await?;
        type DaemonAlias = TestDaemon;
        let daemon = DaemonAlias::new_for_test(data_root, base_url).await?;
        type QualifiedDaemonAlias = ctx_daemon::test_support::TestDaemon;
        let daemon = QualifiedDaemonAlias::new_for_test(data_root, base_url).await?;
        let daemon = test_daemon_for_test(data_root, None).await;
        let daemon = test_daemon_with_fake_provider_for_test(data_root, None).await;
        let app = test_router(state);
        crate::api::router(crate::api::RouteHandles::from_daemon_handle(state.handle()))
      }
    `,
    patterns: LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct lib-test raw TestDaemon construction",
      "direct lib-test raw TestDaemon construction",
      "direct lib-test raw TestDaemon construction",
      "direct lib-test raw TestDaemon construction",
      "direct lib-test raw TestDaemon construction",
      "direct lib-test legacy daemon helper",
      "direct lib-test legacy daemon helper",
      "direct lib-test router helper",
      "direct lib-test router composition",
    ],
  );
});

test("daemon boundary guard allows lib-test data-root daemon fixtures", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    contents: `
      async fn helper(data_root: &Path) {
        let fixture = test_daemon_fixture_with_fake_provider_for_test(data_root, None).await;
        let daemon = fixture.daemon();
        let app = fixture.router();
        daemon.mobile_access_for_test();
      }
    `,
    patterns: LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard scopes lib-test data-root fixture roots", () => {
  assert.deepEqual(
    libTestDataRootFixturePatternsForPath("core/crates/ctx-http/src/lib_tests.rs"),
    LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS,
  );
  assert.deepEqual(
    libTestDataRootFixturePatternsForPath(
      "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/fixtures.rs",
    ),
    LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS,
  );
  assert.deepEqual(
    libTestDataRootFixturePatternsForPath("core/crates/ctx-http/tests/common/mod.rs"),
    [],
  );
});

test("daemon boundary guard scopes task-lifecycle store facade roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/tasks/cleanup_lifecycle_tests.rs",
    "core/crates/ctx-http/src/api/tasks/lifecycle_tests.rs",
    "core/crates/ctx-http/src/api/tasks/lifecycle_tests/fixtures.rs",
    "core/crates/ctx-http/src/api/tasks/lifecycle_tests/delete/subagent_worktree.rs",
    "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
    "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs",
  ]) {
    assert.deepEqual(
      taskLifecycleStorePatternsForPath(filePath),
      TASK_LIFECYCLE_TEST_STORE_ACCESS_PATTERNS,
    );
  }
  assert.deepEqual(taskLifecycleStorePatternsForPath("core/crates/ctx-http/src/api/settings.rs"), []);
});

test("daemon boundary guard scopes storage-admission fixture roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
    "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs",
  ]) {
    assert.deepEqual(
      storageAdmissionFixturePatternsForPath(filePath),
      STORAGE_ADMISSION_FIXTURE_PATTERNS,
    );
  }
  assert.deepEqual(
    storageAdmissionFixturePatternsForPath(
      "core/crates/ctx-http/src/api/tasks/lifecycle_tests/fixtures.rs",
    ),
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
