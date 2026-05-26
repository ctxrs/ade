const assert = require("node:assert/strict");
const test = require("node:test");

const {
  ACP_CRP_BRIDGE_TOKEN_TEST_STORE_ACCESS_PATTERNS,
  AUTH_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
  CACHE_REHYDRATION_TEST_STORE_ACCESS_PATTERNS,
  DAEMON_EXTRACTION_BLOCKER_PATTERNS,
  APPSTATE_DAEMON_HANDLE_CONSTRUCTION_BASELINE,
  APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE,
  DAEMON_ROUTE_HANDLES_ALLOWED_PATHS,
  DELETED_BROAD_DOMAIN_HANDLE_NAMES,
  daemonTestSurfaceRustFiles,
  EXECUTION_HANDLE_ROUTE_EXTRACTOR_ALLOWED_PATHS,
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
  CTX_HTTP_MAIN_DAEMON_INIT_PATTERNS,
  DAEMON_HEALTH_VERSION_PATTERNS,
  DAEMON_UPDATES_VERSION_PATTERNS,
  BLOB_API_ORCHESTRATION_PATTERNS,
  HEALTH_DIAGNOSTICS_API_ORCHESTRATION_PATTERNS,
  RESOURCE_UTILIZATION_API_ROUTE_CONTRACT_PATTERNS,
  SETTINGS_API_ORCHESTRATION_PATTERNS,
  TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
  TELEMETRY_API_ORCHESTRATION_PATTERNS,
  LOGS_API_ORCHESTRATION_PATTERNS,
  UPDATE_API_ORCHESTRATION_PATTERNS,
  UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS,
  ROUTE_FILE_DOWNLOAD_API_PATTERNS,
  ROUTE_DTO_SWEEP_API_PATTERNS,
  SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS,
  RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS,
  WEB_SESSION_ACCESS_ERROR_API_PATTERNS,
  WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS,
  WORKSPACE_CONFIG_ROUTE_CONTEXT_PATTERNS,
  WORKSPACE_EXECUTION_CONFIG_API_PATTERNS,
  WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS,
  WORKSPACE_ROUTE_CONTRACT_API_PATTERNS,
  WORKSPACE_REGISTRATION_CONFIG_API_PATTERNS,
  IMAGE_ATTACHMENTS_TEST_STORE_ACCESS_PATTERNS,
  JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS,
  LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS,
  LIVE_PROVIDER_CANARY_TEST_STORE_ACCESS_PATTERNS,
  MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS,
  CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS,
  CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS,
  CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS,
  PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS,
  PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS,
  DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS,
  MERGE_QUEUE_ISOLATION_TEST_STORE_ACCESS_PATTERNS,
  MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS,
  MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  MOBILE_ACCESS_DAEMON_IMPORT_PATTERNS,
  MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
  MOBILE_ACCESS_ORCHESTRATION_API_PATTERNS,
  MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS,
  MOBILE_TEST_STORE_ACCESS_PATTERNS,
  MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS,
  MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS,
  TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS,
  TASK_ROUTE_API_CONTRACT_PATTERNS,
  TASK_CREATION_PLACEHOLDER_EXTRACTOR_PATTERNS,
  PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS,
  PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS,
  PROVIDER_TEST_HELPER_DAEMON_IMPORT_PATTERNS,
  PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS,
  PROVIDER_HARNESS_CONFIG_API_PATTERNS,
  PROVIDER_HARNESS_ENDPOINT_API_PATTERNS,
  PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS,
  PROVIDER_LAUNCH_AUTH_API_PATTERNS,
  PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
  SESSION_HEAD_API_ORCHESTRATION_PATTERNS,
  SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_SUBAGENT_ROUTE_DAEMON_IMPORT_PATTERNS,
  PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
  PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
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
  mergeQueueEntryApiPatternsForPath,
  mergeQueueSubmitApiPatternsForPath,
  workspaceDeletionCapabilityPresent,
  terminalRestRouteApiPatternsForPath,
  webSessionRestRouteApiPatternsForPath,
  taskRouteApiPatternsForPath,
  titleGenerationApiPatternsForPath,
  mcpDaemonPatternsForPath,
  migratedTestPatternsForPath,
  mobileAccessStoreDtoApiPatternsForPath,
  mobileProfileRouteApiPatternsForPath,
  mobileStorePatternsForPath,
  providerAuthImportApiPatternsForPath,
  providerTestHelperDaemonImportPatternsForPath,
  providerAuthGlobalIdFixturePatternsForPath,
  providerHarnessConfigApiPatternsForPath,
  providerHarnessEndpointApiPatternsForPath,
  providerInstallApiPatternsForPath,
  providerAdminApiPatternsForPath,
  providerLaunchAuthApiPatternsForPath,
  providerLaunchOptionsApiPatternsForPath,
  providerStatusApiPatternsForPath,
  providerUsageApiPatternsForPath,
  providerScenariosOfflineStorePatternsForPath,
  providerWorkerReapingStorePatternsForPath,
  providerCachePatternsForPath,
  providerProbeRuntimeEnvStorePatternsForPath,
  providerRouteSetupStorePatternsForPath,
  providerTargetScopedInstallsStorePatternsForPath,
  replayPropertiesStorePatternsForPath,
  routeFileDownloadApiPatternsForPath,
  sessionArtifactApiPatternsForPath,
  runArchiveApiPatternsForPath,
  routerCompositionPatternsForPath,
  scanAppStateRouteHandleRatchet,
  scanDaemonHandleConstructionRatchet,
  scanDeletedBroadDomainHandleRatchet,
  scanDeletedBroadDomainMacroSourceRatchet,
  scanRouteStateAggregateRatchet,
  scanDaemonShutdownHandleRatchet,
  scanExecutionHandleRouteExtractorRatchet,
  scanTerminalRouteHandleRatchet,
  scanTransportHandleRouteExtractorRatchet,
  scanWebSessionRouteHandleRatchet,
  scanProviderAccountDaemonFacadeRatchet,
  scanProviderAccountHandleRatchet,
  scanProviderAuthImportDaemonFacadeRatchet,
  scanProviderAuthImportHandleRatchet,
  scanProviderBootstrapDaemonFacadeRatchet,
  scanProviderBootstrapHandleRatchet,
  scanProviderHarnessConfigDaemonFacadeRatchet,
  scanProviderHarnessConfigHandleRatchet,
  scanProviderInstallDaemonFacadeRatchet,
  scanProviderInstallHandleRatchet,
  scanProviderLoginDaemonImplementationRatchet,
  scanProviderLoginHandleRatchet,
  scanProviderRuntimeSurfaceDaemonFacadeRatchet,
  scanProviderRuntimeSurfaceHandleRatchet,
  scanProviderRouteHandleDefinitionFiles,
  scanProviderWorkspaceLaunchDaemonFacadeRatchet,
  scanProviderWorkspaceLaunchHandleRatchet,
  scanMergeQueueApiDaemonImplementationRatchet,
  scanMergeQueueApiHandleFieldRatchet,
  scanMergeQueueApiHttpRouteRatchet,
  scanRepoOnboardingDaemonImplementationRatchet,
  scanRepoOnboardingHandleDefinitionFiles,
  scanRepoOnboardingHandleFieldRatchet,
  scanRepoOnboardingRouteExtractorRatchet,
  scanWorkspaceRouteHandleDefinitionFiles,
  scanResourceUtilizationDaemonImplementationRatchet,
  scanResourceUtilizationHandleFieldRatchet,
  scanResourceUtilizationRouteExtractorRatchet,
  scanRunArchiveDaemonImplementationRatchet,
  scanRunArchiveHandleFieldRatchet,
  scanRunArchiveRouteExtractorRatchet,
  scanWorkspaceOrgPolicyDaemonImplementationRatchet,
  scanWorkspaceOrgPolicyHandleFieldRatchet,
  scanWorkspaceOrgPolicyRouteExtractorRatchet,
  scanWorkspacePromptBootstrapConfigDaemonImplementationRatchet,
  scanWorkspacePromptBootstrapConfigHandleFieldRatchet,
  scanWorkspacePromptBootstrapConfigRouteExtractorRatchet,
  scanWorkspaceExecutionConfigDaemonImplementationRatchet,
  scanWorkspaceExecutionConfigHandleFieldRatchet,
  scanWorkspaceExecutionConfigRouteExtractorRatchet,
  scanWorkspaceFileCompletionsDaemonImplementationRatchet,
  scanWorkspaceFileCompletionsHandleFieldRatchet,
  scanWorkspaceFileCompletionsRouteExtractorRatchet,
  scanWorkspaceHarnessContainerDaemonImplementationRatchet,
  scanWorkspaceHarnessContainerHandleFieldRatchet,
  scanWorkspaceHarnessContainerRouteExtractorRatchet,
  scanWorkspaceWorktreeDaemonImplementationRatchet,
  scanWorkspaceWorktreeHandleFieldRatchet,
  scanWorkspaceWorktreeRouteExtractorRatchet,
  scanWorkspaceDeletionDaemonImplementationRatchet,
  scanWorkspaceDeletionHandleFieldRatchet,
  scanWorkspaceDeletionRouteExtractorRatchet,
  scanWorkspaceRegistryDaemonImplementationRatchet,
  scanWorkspaceRegistryHandleFieldRatchet,
  scanWorkspaceRegistryRouteExtractorRatchet,
  scanWorkspaceRestConfigStoreIntentRatchet,
  scanTerminalWebSessionLaunchStoreIntentRatchet,
  scanWorkspaceAttachmentsDaemonImplementationRatchet,
  scanWorkspaceAttachmentsHandleFieldRatchet,
  scanWorkspaceAttachmentsRouteExtractorRatchet,
  scanWorkspaceCompositionDeletedPathRatchet,
  scanWorkspaceCompositionStateCutRatchet,
  scanWorkspaceMergeQueueConfigDaemonImplementationRatchet,
  scanWorkspaceMergeQueueConfigHandleFieldRatchet,
  scanWorkspaceMergeQueueConfigRouteExtractorRatchet,
  scanWorkspacePrimaryBranchDaemonImplementationRatchet,
  scanWorkspacePrimaryBranchHandleFieldRatchet,
  scanWorkspacePrimaryBranchRouteExtractorRatchet,
  scanWorkspaceProviderModelPreferenceDaemonImplementationRatchet,
  scanWorkspaceProviderModelPreferenceHandleFieldRatchet,
  scanWorkspaceProviderModelPreferenceRouteExtractorRatchet,
  scanTaskAdmissionDaemonImplementationRatchet,
  scanTaskAdmissionHandleFieldRatchet,
  scanTaskAdmissionHandleRatchet,
  scanTaskLifecycleDaemonImplementationRatchet,
  scanTaskLifecycleHandleFieldRatchet,
  scanTaskLifecycleHandleRatchet,
  scanTaskReadMetadataDaemonImplementationRatchet,
  scanTaskReadMetadataHandleFieldRatchet,
  scanTaskReadMetadataHandleRatchet,
  scanSessionArtifactsDaemonImplementationRatchet,
  scanSessionArtifactsHandleFieldRatchet,
  scanSessionArtifactsHandleRatchet,
  scanSessionControlAssemblyRatchet,
  scanSessionControlDaemonImplementationRatchet,
  scanSessionControlHandleFieldRatchet,
  scanSessionControlHandleRatchet,
  scanSessionFileCompletionsDaemonSeamRatchet,
  scanSessionFileCompletionsHandleRatchet,
  scanSessionReadModelsDaemonImplementationRatchet,
  scanSessionReadModelsHandleFieldRatchet,
  scanSessionReadModelsHandleRatchet,
  scanSessionTitleModelModeAssemblyRatchet,
  scanSessionTitleModelModeDaemonImplementationRatchet,
  scanSessionTitleModelModeHandleFieldRatchet,
  scanSessionTitleModelModeHandleRatchet,
  scanSessionTitleModelModeTitleImplementationRatchet,
  scanSessionMessageCommandDaemonImplementationRatchet,
  scanSessionMessageCommandHandleFieldRatchet,
  scanSessionMessageCommandHandleRatchet,
  scanSessionSubagentMcpReadDaemonImplementationRatchet,
  scanSessionSubagentMcpReadHandleFieldRatchet,
  scanSessionSubagentMcpReadHandleRatchet,
  scanSessionSubagentMcpControlDaemonImplementationRatchet,
  scanSessionSubagentMcpControlHandleFieldRatchet,
  scanSessionSubagentMcpControlHandleRatchet,
  scanSubagentSpawnHostStateRatchet,
  scanSchedulerHostStateRatchet,
  scanTaskSessionPublicationEffectsStateRatchet,
  scanFinalSessionRouteCapabilityRatchet,
  scanSessionSubagentReadDaemonImplementationRatchet,
  scanSessionSubagentReadHandleFieldRatchet,
  scanSessionSubagentReadHandleRatchet,
  scanSessionVcsDaemonImplementationRatchet,
  scanSessionVcsHandleFieldRatchet,
  scanSessionVcsHandleRatchet,
  scanWorkspaceActiveAssemblyRatchet,
  scanWorkspaceActiveDaemonImplementationRatchet,
  scanWorkspaceActiveHandleFieldRatchet,
  scanWorkspaceActiveRouteExtractorRatchet,
  scanWorkspaceStreamAssemblyRatchet,
  scanWorkspaceStreamActiveDaemonImplementationRatchet,
  scanWorkspaceStreamHandleFieldRatchet,
  scanWorkspaceStreamRouteExtractorRatchet,
  scanWorktreeVcsAssemblyRatchet,
  scanWorktreeVcsExplicitHostImplementationRatchet,
  scanWorkspaceVcsStreamDaemonImplementationRatchet,
  scanWorkspaceVcsStreamHandleFieldRatchet,
  scanWorkspaceVcsStreamRouteExtractorRatchet,
  scanRepo,
  scanRouteCapabilityCutoverRatchet,
  scanRouterComposition,
  scanText,
  schedulerRuntimeStorePatternsForPath,
  sessionHeadApiPatternsForPath,
  sessionControlRouteApiPatternsForPath,
  sessionMessageCommandRouteApiPatternsForPath,
  sessionReadModelRouteApiPatternsForPath,
  demoSeedTranscriptApiPatternsForPath,
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
  updateDrainApiPatternsForPath,
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

test("appstate route handle ratchet rejects new full-state handle families", () => {
  assert.equal(APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE.size, 0);
  assert.equal(APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE.has("CoreHandle"), false);
  assert.equal(APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE.has("TasksHandle"), false);
  assert.equal(APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE.has("TelemetryHandle"), false);
  assert.equal(APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE.has("WorkspaceStreamHandle"), false);
  assert.equal(APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE.has("TransportHandle"), false);
  assert.equal(APPSTATE_FULL_STATE_DOMAIN_HANDLE_BASELINE.has("ExecutionHandle"), false);
  const violations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      use super::state::DaemonState;
      macro_rules! domain_handle_with_accessor {
        ($name:ident, $accessor:ident) => {
          pub struct $name {
            state: Arc<DaemonState>,
          }
        };
      }
      domain_handle_with_accessor!(SurpriseHandle, surprise);
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "full-state route handle ratchet exceeded",
      "unclassified full-state route handle",
    ],
  );
});

test("appstate route handle ratchet rejects stale transport and execution handles", () => {
  const transportViolations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      use super::state::DaemonState;
      macro_rules! domain_handle_with_accessor {
        ($name:ident, $accessor:ident) => {
          pub struct $name {
            state: Arc<DaemonState>,
          }
        };
      }
      domain_handle_with_accessor!(TransportHandle, transport);
    `,
  });
  assert.deepEqual(
    transportViolations.map((violation) => violation.name),
    [
      "full-state route handle ratchet exceeded",
      "unclassified full-state route handle",
    ],
  );
  assert.deepEqual(transportViolations.at(-1).text, "TransportHandle");

  const executionViolations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      use super::state::DaemonState;
      macro_rules! domain_handle_with_accessor {
        ($name:ident, $accessor:ident) => {
          pub struct $name {
            state: Arc<DaemonState>,
          }
        };
      }
      domain_handle_with_accessor!(ExecutionHandle, execution);
    `,
  });
  assert.deepEqual(
    executionViolations.map((violation) => violation.name),
    [
      "full-state route handle ratchet exceeded",
      "unclassified full-state route handle",
    ],
  );
  assert.deepEqual(executionViolations.at(-1).text, "ExecutionHandle");
});

test("appstate route handle ratchet rejects migrated telemetry full-state reintroduction", () => {
  const violations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      use super::state::DaemonState;
      macro_rules! domain_handle_with_accessor {
        ($name:ident, $accessor:ident) => {
          pub struct $name {
            state: Arc<DaemonState>,
          }
        };
      }
      domain_handle_with_accessor!(TelemetryHandle, telemetry);
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "full-state route handle ratchet exceeded",
      "unclassified full-state route handle",
    ],
  );
});

test("appstate route handle ratchet rejects stale tasks full-state reintroduction", () => {
  const violations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      use super::state::DaemonState;
      macro_rules! domain_handle_with_accessor {
        ($name:ident, $accessor:ident) => {
          pub struct $name {
            state: Arc<DaemonState>,
          }
        };
      }
      domain_handle_with_accessor!(TasksHandle, tasks);
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "full-state route handle ratchet exceeded",
      "unclassified full-state route handle",
    ],
  );
  assert.deepEqual(violations.at(-1).text, "TasksHandle");
});

test("appstate route handle ratchet rejects workspace stream full-state reintroduction", () => {
  const violations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      use super::state::DaemonState;
      macro_rules! domain_handle_with_accessor {
        ($name:ident, $accessor:ident) => {
          pub struct $name {
            state: Arc<DaemonState>,
          }
        };
      }
      domain_handle_with_accessor!(WorkspaceStreamHandle, workspace_stream);
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "full-state route handle ratchet exceeded",
      "unclassified full-state route handle",
    ],
  );
  assert.deepEqual(violations.at(-1).text, "WorkspaceStreamHandle");
});

test("appstate route handle ratchet rejects direct full-state route handles", () => {
  const violations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      use super::state::DaemonState;
      pub struct DaemonHandle {
        state: Arc<DaemonState>,
      }
      pub struct SurpriseHandle {
        state: Arc<DaemonState>,
      }
      pub struct OtherHandleParts {
        state: std::sync::Arc<crate::daemon::state::DaemonState>,
      }
      pub struct CallbackHandleEffects {
        daemon: Arc<DaemonHandle>,
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "direct full-state route handle",
      "direct full-state route handle",
      "direct full-state route handle",
    ],
  );
});

test("appstate route handle ratchet rejects parts outside central handle file", () => {
  const violations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/session_control_effects.rs",
    contents: `
      use std::sync::Arc;
      use crate::daemon::state::DaemonState;

      pub(in crate::daemon) struct SessionControlHandleParts {
        state: Arc<DaemonState>,
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["direct full-state route handle"],
  );
});

test("appstate route handle ratchet rejects aliases and broad return values", () => {
  const violations = scanAppStateRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub type AuthHandle = std::sync::Arc<crate::daemon::state::DaemonState>;

      pub struct HealthHandle {
        store: Store,
      }

      impl HealthHandle {
        pub fn leak_state(&self) -> anyhow::Result<Arc<DaemonState>> {
          todo!()
        }

        pub fn leak_handle(&self) -> DaemonHandle {
          todo!()
        }
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "route handle aliases broad daemon state",
      "route handle returns broad daemon state",
      "route handle returns broad daemon state",
    ],
  );
});

test("deleted broad domain handle ratchet rejects final AppState escape hatches", () => {
  assert.deepEqual(
    [...DELETED_BROAD_DOMAIN_HANDLE_NAMES].sort(),
    ["ProvidersHandle", "SessionsHandle", "WorkspacesHandle"],
  );

  const violations = scanDeletedBroadDomainHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      macro_rules! domain_handle_with_accessor { ($name:ident, $accessor:ident) => {} }
      domain_handle_with_accessor!(SessionsHandle, sessions);
      pub fn sessions(&self) -> SessionsHandle { todo!() }
      pub fn workspaces(&self) -> WorkspacesHandle { todo!() }
      pub fn providers(&self) -> ProvidersHandle { todo!() }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "deleted broad domain handle",
      "deleted broad domain handle",
      "deleted broad domain handle",
      "deleted broad domain handle",
      "deleted broad domain handle macro",
      "deleted broad domain handle macro",
    ],
  );
});

test("deleted broad domain handle ratchet scans daemon test support surfaces", () => {
  assert.equal(
    daemonTestSurfaceRustFiles().some((filePath) =>
      filePath.endsWith("core/crates/ctx-daemon/src/test_support.rs"),
    ),
    true,
  );
});

test("deleted broad domain macro source ratchet scans source roots only", () => {
  assert.deepEqual(scanDeletedBroadDomainMacroSourceRatchet(), []);
});

test("route state aggregate ratchet rejects broad route aggregate extraction", () => {
  const violations = scanRouteStateAggregateRatchet({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      use axum::extract::{FromRef, State};
      use crate::api::router::{RouteHandles, RouteState};

      async fn route(State(state): State<RouteState>) {
        let _ = state.handles.auth.clone();
      }

      async fn route_handles(State(handles): State<RouteHandles>) {
        let _ = handles;
      }

      async fn route_absolute(
        State(route_state): State<::crate::api::router::RouteState>,
      ) {
        let _ = route_state;
      }

      impl FromRef<RouteState> for RouteHandles {
        fn from_ref(state: &RouteState) -> Self {
          state.handles.clone()
        }
      }

      impl axum::extract::FromRef<RouteState> for crate::api::router::RouteHandles {
        fn from_ref(state: &RouteState) -> Self {
          state.handles.clone()
        }
      }

      impl ::axum::extract::FromRef<::crate::api::router::RouteState>
        for ::crate::api::router::RouteHandles
      {
        fn from_ref(route_state: &::crate::api::router::RouteState) -> Self {
          route_state.handles.clone()
        }
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "broad route aggregate extractor",
      "broad route aggregate extractor",
      "broad route aggregate extractor",
      "broad route aggregate FromRef",
      "broad route aggregate FromRef",
      "broad route aggregate FromRef",
      "direct route aggregate handles access",
      "direct route aggregate handles access",
      "direct route aggregate handles access",
      "direct route aggregate handles access",
    ],
  );
});

test("route state aggregate ratchet does not reject unrelated handles fields", () => {
  const violations = scanRouteStateAggregateRatchet({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      struct Fixture {
        handles: Vec<String>,
      }

      fn helper(fixture: Fixture) {
        let _ = fixture.handles.clone();
      }

      fn helper_with_state_name(state: Fixture) {
        let _ = state.handles.clone();
      }

      fn helper_with_route_state_name(route_state: Fixture) {
        let _ = route_state.handles.clone();
      }
    `,
  });

  assert.deepEqual(violations, []);
});

test("route state aggregate ratchet rejects public aggregate fields", () => {
  const violations = scanRouteStateAggregateRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      pub struct RouteHandles {
        pub(in crate::api) auth: AuthHandle,
        pub(crate) health: HealthHandle,
        diagnostics: DiagnosticsHandle,
      }

      pub(in crate::api) struct RouteState {
        pub handles: RouteHandles,
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "public route aggregate field",
      "public route aggregate field",
      "public route aggregate field",
    ],
  );
});

test("route state aggregate ratchet allows router composition and router-local access", () => {
  const routeCompositionViolations = scanRouteStateAggregateRatchet({
    filePath: "core/crates/ctx-http/src/api/routes.rs",
    contents: `
      use crate::api::router::RouteState;

      pub(super) fn api_routes() -> axum::Router<RouteState> {
        axum::Router::new()
      }
    `,
  });
  assert.deepEqual(routeCompositionViolations, []);

  const routerViolations = scanRouteStateAggregateRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      pub struct RouteHandles {
        auth: AuthHandle,
      }

      pub(in crate::api) struct RouteState {
        handles: RouteHandles,
      }

      impl FromRef<RouteState> for AuthHandle {
        fn from_ref(state: &RouteState) -> Self {
          state.handles.auth.clone()
        }
      }
    `,
  });
  assert.deepEqual(routerViolations, []);
});

test("route capability cutover ratchet rejects retired DaemonHandle router assembly", () => {
  const violations = scanRouteCapabilityCutoverRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        pub fn from_daemon_handle(handle: DaemonHandle) -> Self {
          RouteHandles::from_daemon_handle(handle)
        }
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "retired RouteHandles::from_daemon_handle",
      "retired RouteHandles::from_daemon_handle",
      "ctx-http DaemonHandle usage",
    ],
  );
});

test("route capability cutover ratchet rejects retired daemon route handle assembly", () => {
  const violations = scanRouteCapabilityCutoverRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/route_capabilities.rs",
    contents: `
      impl DaemonRouteHandles {
        pub(in crate::daemon) fn from_handle(handle: &DaemonHandle) -> Self {
          DaemonRouteHandles::from_handle(handle)
        }
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "retired DaemonRouteHandles::from_handle",
      "retired DaemonRouteHandles::from_handle",
    ],
  );
});

test("route capability cutover ratchet rejects DaemonHandle throughout ctx-http surfaces", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/server.rs",
    "core/crates/ctx-http/tests/common/mod.rs",
    "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs",
  ]) {
    const violations = scanRouteCapabilityCutoverRatchet({
      filePath,
      contents: "use ctx_daemon::daemon::DaemonHandle;",
    });
    assert.deepEqual(
      violations.map((violation) => violation.name),
      ["ctx-http DaemonHandle usage"],
    );
  }
});

test("route capability cutover ratchet restricts DaemonRouteHandles to router assembly files", () => {
  assert.deepEqual(
    [...DAEMON_ROUTE_HANDLES_ALLOWED_PATHS].sort(),
    [
      "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs",
      "core/crates/ctx-http/src/api/router.rs",
      "core/crates/ctx-http/src/server.rs",
      "core/crates/ctx-http/src/test_support.rs",
      "core/crates/ctx-http/tests/common/mod.rs",
    ],
  );

  const violations = scanRouteCapabilityCutoverRatchet({
    filePath: "core/crates/ctx-http/src/api/tasks.rs",
    contents: `
      use ctx_daemon::daemon::DaemonRouteHandles;
      fn build(handles: DaemonRouteHandles) {}
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "DaemonRouteHandles outside router assembly",
      "DaemonRouteHandles outside router assembly",
    ],
  );
});

test("route capability cutover ratchet rejects DaemonRouteHandles field access outside constructor", () => {
  const violations = scanRouteCapabilityCutoverRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      use ctx_daemon::daemon::DaemonRouteHandles;

      impl RouteHandles {
        pub fn from_daemon_route_handles(handles: DaemonRouteHandles) -> Self {
          let DaemonRouteHandles { auth } = handles;
          Self { auth }
        }

        fn leak(handles: DaemonRouteHandles) {
          let _ = handles.auth.clone();
          let DaemonRouteHandles { auth } = handles;
          let _ = auth;
        }

        fn leak_inferred(runtime: daemon::DaemonRuntime, daemon: TestDaemon) {
          let daemon::DaemonRuntime { route_handles, .. } = runtime;
          let _ = route_handles.auth.clone();
          let leaked = daemon.route_handles();
          let _ = leaked.auth.clone();
          let daemon::DaemonRuntime { route_handles: renamed, .. } = runtime;
          let _ = renamed.auth.clone();
          let _ = daemon.route_handles().auth.clone();
          let _ = route_handles
            .auth
            .clone();
          let _ = daemon.route_handles()
            .auth
            .clone();
          let leaked_multiline = daemon
            .route_handles();
          let _ = leaked_multiline.auth.clone();
        }
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "DaemonRouteHandles destructuring outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
      "direct DaemonRouteHandles field access outside RouteHandles constructor",
    ],
  );
});

test("route capability cutover ratchet allows the RouteHandles constructor handoff", () => {
  const violations = scanRouteCapabilityCutoverRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      use ctx_daemon::daemon::DaemonRouteHandles;

      impl RouteHandles {
        pub fn from_daemon_route_handles(handles: DaemonRouteHandles) -> Self {
          let DaemonRouteHandles { auth } = handles;
          Self { auth }
        }
      }
    `,
  });

  assert.deepEqual(violations, []);
});

test("appstate daemon handle construction ratchet rejects new production reconstructions", () => {
  assert.equal(APPSTATE_DAEMON_HANDLE_CONSTRUCTION_BASELINE.length, 1);
  const violations = scanDaemonHandleConstructionRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/tasks/other.rs",
    contents: `
      use crate::daemon::DaemonHandle;
      fn rebuild(tasks: TasksHandle) {
        let daemon = DaemonHandle::new(tasks.state.clone());
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "unclassified daemon handle surface",
      "unclassified daemon handle reconstruction",
    ],
  );
});

test("appstate daemon handle construction ratchet rejects broad production surfaces", () => {
  const violations = scanDaemonHandleConstructionRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/tasks/other.rs",
    contents: `
      use crate::daemon::DaemonHandle;

      pub struct TaskHelper {
        handle: Option<DaemonHandle>,
      }

      pub fn build(handle: Arc<DaemonHandle>) -> Result<DaemonHandle> {
        todo!()
      }

      pub fn callback() -> Box<dyn Fn() -> DaemonHandle> {
        todo!()
      }

      impl From<TaskHelper> for DaemonHandle {
        fn from(value: TaskHelper) -> Self {
          todo!()
        }
      }

      fn literal() {
        let _ = DaemonHandle { state };
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "unclassified daemon handle surface",
      "unclassified daemon handle surface",
      "unclassified daemon handle surface",
      "unclassified daemon handle surface",
      "unclassified daemon handle surface",
      "unclassified daemon handle surface",
    ],
  );
});

test("appstate daemon handle construction ratchet rejects From and into escape hatches", () => {
  const violations = scanDaemonHandleConstructionRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/tasks/other.rs",
    contents: `
      fn rebuild(tasks: TasksHandle) {
        let daemon = DaemonHandle::from(tasks.state.clone());
        let multiline_from = DaemonHandle::from
          (tasks.state.clone());
        let multiline_new = DaemonHandle::new
          (tasks.state.clone());
        let other:
          DaemonHandle =
          tasks.state.clone().into();
        consume(tasks.state.clone().into());
        let arc_clone: DaemonHandle = Arc::clone(&tasks.state).into();
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "unclassified daemon handle surface",
      "unclassified daemon handle surface",
      "unclassified daemon handle reconstruction",
      "unclassified daemon handle reconstruction",
      "unclassified daemon handle reconstruction",
      "unclassified daemon handle reconstruction",
      "unclassified daemon handle reconstruction",
      "unclassified daemon handle reconstruction",
    ],
  );
});

test("appstate daemon handle construction ratchet rejects Arc<DaemonState> From impl", () => {
  const violations = scanDaemonHandleConstructionRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      use std::sync::Arc;
      impl From<Arc<DaemonState>> for DaemonHandle {
        fn from(state: Arc<DaemonState>) -> Self {
          Self::new(state)
        }
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["daemon handle Arc<DaemonState> From impl"],
  );
});

test("appstate daemon handle construction ratchet preserves known baseline reconstructions", () => {
  const violations = scanDaemonHandleConstructionRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: "let handle = DaemonHandle::new(Arc::clone(state));",
  });

  assert.deepEqual(violations, []);
});

test("appstate execution handle extractor ratchet rejects all HTTP routes", () => {
  assert.equal(
    EXECUTION_HANDLE_ROUTE_EXTRACTOR_ALLOWED_PATHS.has(
      "core/crates/ctx-http/src/api/updates/drain/shutdown.rs",
    ),
    false,
  );
  const violations = scanExecutionHandleRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/execution.rs",
    contents: `
      use ctx_daemon::daemon::ExecutionHandle;
      async fn route(State(execution): State<ExecutionHandle>) {
        execution.start_execution_launch_for_request(req).await;
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "execution route extracts broad execution handle",
      "execution route extracts broad execution handle",
    ],
  );

  assert.deepEqual(
    scanExecutionHandleRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/updates/drain/shutdown.rs",
      contents: `
        use ctx_daemon::daemon::ExecutionHandle;
        async fn route(State(execution): State<ExecutionHandle>) {}
      `,
    }).map((violation) => violation.name),
    [
      "execution route extracts broad execution handle",
      "execution route extracts broad execution handle",
    ],
  );
});

test("appstate daemon shutdown handle ratchet rejects broad fields and wrong route extraction", () => {
  const routeViolations = scanDaemonShutdownHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/execution.rs",
    contents: `
      async fn route(State(shutdown): State<DaemonShutdownHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routeViolations, [
    "daemon shutdown route extracts DaemonShutdownHandle outside shutdown route",
  ]);

  assert.deepEqual(
    scanDaemonShutdownHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/updates/drain/shutdown.rs",
      contents: `
        async fn route(State(shutdown): State<DaemonShutdownHandle>) {}
      `,
    }),
    [],
  );

  const fieldViolations = scanDaemonShutdownHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      type DaemonShutdownEffect = Arc<dyn Fn(String) -> DaemonShutdownFuture + Send + Sync>;

      impl DaemonHandle {
        pub fn daemon_shutdown(&self) -> DaemonShutdownHandle {
          let state = Arc::clone(&self.state);
        }
      }

      pub struct DaemonShutdownHandle {
        state: Arc<DaemonState>,
        request_shutdown: Arc<dyn Fn(String) -> DaemonShutdownFuture + Send + Sync>,
        execution: ExecutionHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "daemon shutdown capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "daemon shutdown capability exposes generic full-state escape hatch",
    ),
  );
  assert(
    fieldViolations.includes(
      "daemon shutdown capability uses callback effect seam",
    ),
  );
  assert(
    fieldViolations.includes(
      "daemon shutdown capability stores callback effect seam",
    ),
  );
  assert(
    fieldViolations.includes("daemon shutdown assembly clones full daemon state"),
  );

  const hostViolations = scanDaemonShutdownHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/lifecycle/shutdown_host.rs",
    contents: `
      pub(in crate::daemon) struct DaemonShutdownHost {
        state: Arc<DaemonState>,
        handle: DaemonHandle,
        request_shutdown: Arc<dyn Fn(String) -> DaemonShutdownFuture + Send + Sync>,
      }

      pub(in crate::daemon) struct DaemonShutdownHostParts {
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    hostViolations.includes(
      "daemon shutdown host stores broad handle or daemon state",
    ),
  );
  assert(
    hostViolations.includes(
      "daemon shutdown host exposes generic full-state escape hatch",
    ),
  );
  assert(
    hostViolations.includes("daemon shutdown host stores callback effect seam"),
  );
});

test("appstate terminal route handle ratchet rejects broad terminal route seams", () => {
  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/tasks.rs",
      contents: `
        async fn route(State(terminals): State<TerminalRouteHandle>) {}
      `,
    }).map((violation) => violation.name),
    ["terminal route extracts TerminalRouteHandle outside terminal routes"],
  );

  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/terminals.rs",
      contents: `
        use ctx_daemon::daemon::TransportHandle;
        async fn route(State(transport): State<TransportHandle>) {}
      `,
    }).map((violation) => violation.name),
    [
      "terminal route extracts broad transport handle",
      "terminal route extracts broad transport handle",
    ],
  );

  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/terminals/route_contract.rs",
      contents: `
        impl TransportHandle {
          pub async fn delete_terminal_for_route(&self) {}
        }
      `,
    }).map((violation) => violation.name),
    ["terminal route contract remains on broad transport handle"],
  );

  const fieldViolations = scanTerminalRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct TerminalRouteHandle {
        state: Arc<DaemonState>,
        transport: TransportHandle,
        web: WebSessionRouteHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "terminal route capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "terminal route capability exposes generic full-state escape hatch",
    ),
  );
  assert(
    fieldViolations.includes(
      "terminal route capability exposes unexpected production field",
    ),
  );
  assert.equal(
    fieldViolations.filter(
      (name) => name === "terminal route capability stores broad handle or daemon state",
    ).length,
    3,
  );

  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/ws/terminal.rs",
      contents: `
        async fn route(State(terminals): State<TerminalRouteHandle>) {}
      `,
    }),
    [],
  );
});

test("appstate terminal route handle ratchet rejects state-capturing terminal launch assembly", () => {
  const violations = scanTerminalRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn terminal_route(&self) -> TerminalRouteHandle {
          let create_terminal = Arc::new({
            let state = Arc::clone(&self.state);
            move |req: CreateTerminalLaunchRequest| {
              let state = Arc::clone(&state);
              Box::pin(async move {
                crate::daemon::terminals::create_workspace_terminal(&state, req).await
              }) as CreateTerminalFuture
            }
          });
          TerminalRouteHandle::new(
            Arc::clone(&self.state.transport.terminals),
            create_terminal,
          )
        }

        pub fn web_session_route(&self) -> WebSessionRouteHandle {
          let state = Arc::clone(&self.state);
          WebSessionRouteHandle::new(state)
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("terminal route assembly captures full daemon state"),
  );
  assert(
    violations.includes(
      "terminal route assembly uses state-capturing terminal launch closure",
    ),
  );
  assert.equal(
    violations.filter(
      (name) => name === "terminal route assembly captures full daemon state",
    ).length,
    1,
  );
});

test("appstate terminal route handle ratchet allows typed terminal launch host assembly", () => {
  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        impl DaemonHandle {
          pub fn terminal_route(&self) -> TerminalRouteHandle {
            TerminalRouteHandle::new(
              Arc::clone(&self.state.transport.terminals),
              self.terminal_launch_host(),
            )
          }
        }

        #[cfg(test)]
        pub(in crate::daemon) type CreateTerminalFuture =
          Pin<Box<dyn Future<Output = Result<TerminalSession, TerminalLaunchError>> + Send + 'static>>;
        #[cfg(test)]
        pub(in crate::daemon) type CreateTerminalEffect =
          Arc<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>;

        #[derive(Clone)]
        enum TerminalRouteLaunch {
          Host(TerminalLaunchHost),
          #[cfg(test)]
          Override(CreateTerminalEffect),
        }

        pub struct TerminalRouteHandle {
          terminals: Arc<TerminalManager>,
          launch: TerminalRouteLaunch,
        }
      `,
    }),
    [],
  );
});

test("appstate terminal route handle ratchet rejects production callback storage", () => {
  const violations = scanTerminalRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub(in crate::daemon) type CreateTerminalFuture =
        Pin<Box<dyn Future<Output = Result<TerminalSession, TerminalLaunchError>> + Send + 'static>>;
      pub(in crate::daemon) type CreateTerminalEffect =
        Arc<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>;
      type TerminalLauncher =
        Box<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>;

      pub struct TerminalRouteHandle {
        terminals: Arc<TerminalManager>,
        create_terminal: CreateTerminalEffect,
        create_terminal_alias: TerminalLauncher,
        create_terminal_direct:
          Arc<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>,
        create_terminal_boxed:
          Box<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>,
        create_terminal_fn: fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("terminal route production callback effect alias remains"),
  );
  assert(
    violations.includes("terminal route capability stores production callback effect"),
  );
  assert(
    violations.includes("terminal route capability stores production callback shape"),
  );
  assert(
    violations.includes("terminal route production callback-shaped alias remains"),
  );
  assert(
    violations.includes(
      "terminal route capability exposes unexpected production field",
    ),
  );
});

test("appstate terminal route handle ratchet rejects production launch enum backdoors", () => {
  const violations = scanTerminalRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      type TerminalLauncher =
        Box<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>;

      enum TerminalRouteLaunch {
        Host(TerminalLaunchHost),
        Callback(Arc<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>),
        BoxedCallback(Box<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>),
        AliasCallback(TerminalLauncher),
        Broad(FooDomainHandle),
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes(
      "terminal route launch enum stores production callback effect",
    ),
  );
  assert(
    violations.includes(
      "terminal route launch enum stores broad handle or daemon state",
    ),
  );
  assert(
    violations.includes("terminal route production callback-shaped alias remains"),
  );
  assert(
    violations.includes(
      "terminal route launch enum exposes unexpected production variant",
    ),
  );
});

test("appstate terminal route handle ratchet allows test-gated callback alias use", () => {
  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        type TerminalLauncher =
          Box<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>;

        enum TerminalRouteLaunch {
          Host(TerminalLaunchHost),
          #[cfg(test)]
          Override(TerminalLauncher),
        }

        pub struct TerminalRouteHandle {
          terminals: Arc<TerminalManager>,
          launch: TerminalRouteLaunch,
          #[cfg(test)]
          override_launch: TerminalLauncher,
        }
      `,
    }),
    [],
  );
});

test("appstate terminal launch ratchet rejects DaemonState in production launch files", () => {
  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch/worktree.rs",
      contents: `
        use crate::daemon::DaemonState;

        async fn resolve(state: Arc<DaemonState>) {}
      `,
    }).map((violation) => violation.name),
    [
      "terminal launch production helper mentions DaemonState",
      "terminal launch production helper mentions DaemonState",
    ],
  );

  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch/tests.rs",
      contents: `
        use crate::daemon::DaemonState;

        async fn test_state() -> Arc<DaemonState> {}
      `,
    }),
    [],
  );
});

test("appstate terminal launch ratchet rejects broad handles in production launch files", () => {
  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch/worktree.rs",
      contents: `
        use crate::daemon::{DaemonHandle, FooDomainHandle};

        async fn resolve(handle: &DaemonHandle, domain: &FooDomainHandle) {}
      `,
    }).map((violation) => violation.name),
    [
      "terminal launch production helper mentions broad daemon or route handle",
      "terminal launch production helper mentions broad daemon or route handle",
      "terminal launch production helper mentions broad daemon or route handle",
      "terminal launch production helper mentions broad daemon or route handle",
    ],
  );
});

test("appstate terminal launch ratchet rejects broad host fields", () => {
  const violations = scanTerminalRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch.rs",
    contents: `
      pub(in crate::daemon) struct TerminalLaunchHost {
        state: Arc<DaemonState>,
        route: TerminalRouteHandle,
        domain: FooDomainHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes(
      "terminal launch production helper mentions DaemonState",
    ),
  );
  assert(
    violations.includes("terminal launch host stores broad daemon or route handle"),
  );
  assert(
    violations.includes(
      "terminal launch host exposes generic full-state escape hatch",
    ),
  );
});

test("appstate terminal launch ratchet allows narrow host dependencies", () => {
  assert.deepEqual(
    scanTerminalRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch.rs",
      contents: `
        pub(in crate::daemon) struct TerminalLaunchHost {
          global_store: Store,
          workspace_stores: ProtectedWorkspaceStoreLookup,
          data_root: PathBuf,
          daemon_url: String,
          harness: Arc<HarnessRuntimeManager>,
          terminals: Arc<TerminalManager>,
        }
      `,
    }),
    [],
  );
});

test("appstate transport handle ratchet rejects production HTTP API transport handles", () => {
  assert.deepEqual(
    scanTransportHandleRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/web_sessions/actions.rs",
      contents: `
        use ctx_daemon::daemon::TransportHandle;
        async fn route(State(transport): State<TransportHandle>) {}
      `,
    }).map((violation) => violation.name),
    [
      "HTTP API route exposes broad transport handle",
      "HTTP API route exposes broad transport handle",
    ],
  );

  assert.deepEqual(
    scanTransportHandleRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/test_support.rs",
      contents: "use ctx_daemon::daemon::TransportHandle;",
    }),
    [],
  );
});

test("appstate web-session route handle ratchet rejects broad route seams", () => {
  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/web_sessions/route_contract.rs",
      contents: `
        impl TransportHandle {
          pub async fn run_web_session_for_route(&self) {}
        }
      `,
    }).map((violation) => violation.name),
    ["web-session route contract remains on broad transport handle"],
  );

  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/web_sessions.rs",
      contents: `
        impl TransportHandle {
          pub async fn prepare_web_session_view_page(&self) {}
        }
      `,
    }).map((violation) => violation.name),
    ["web-session route contract remains on broad transport handle"],
  );

  const fieldViolations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WebSessionRouteHandle {
        state: Arc<DaemonState>,
        effects: Effects,
        transport: TransportHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "web-session route capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "web-session route capability exposes generic full-state escape hatch",
    ),
  );
  assert(
    fieldViolations.includes(
      "web-session route capability exposes unexpected production field",
    ),
  );
});

test("appstate web-session route handle ratchet rejects state-capturing launch assembly", () => {
  const violations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn web_session_route(&self) -> WebSessionRouteHandle {
          let create_web_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |req: WebSessionLaunchRequest| {
              let state = Arc::clone(&state);
              Box::pin(async move {
                crate::daemon::web_sessions::create_web_session(&state, req).await
              }) as CreateWebSessionFuture
            }
          });
          WebSessionRouteHandle::new(
            Arc::clone(&self.state.transport.web_sessions),
            create_web_session,
          )
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("web-session route assembly captures full daemon state"),
  );
  assert(
    violations.includes(
      "web-session route assembly uses state-capturing launch closure",
    ),
  );
});

test("appstate web-session route handle ratchet allows typed launch host assembly", () => {
  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        impl DaemonHandle {
          pub fn web_session_route(&self) -> WebSessionRouteHandle {
            WebSessionRouteHandle::new(
              Arc::clone(&self.state.transport.web_sessions),
              self.web_session_launch_host(),
            )
          }
        }

        #[cfg(test)]
        pub(in crate::daemon) type CreateWebSessionFuture =
          Pin<Box<dyn Future<Output = Result<WebSessionInfo, WebSessionLaunchError>> + Send + 'static>>;
        #[cfg(test)]
        pub(in crate::daemon) type CreateWebSessionEffect =
          Arc<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>;

        #[derive(Clone)]
        enum WebSessionRouteLaunch {
          Host(WebSessionLaunchHost),
          #[cfg(test)]
          Override(CreateWebSessionEffect),
        }

        pub struct WebSessionRouteHandle {
          web_sessions: Arc<WebSessionManager>,
          launch: WebSessionRouteLaunch,
        }
      `,
    }),
    [],
  );
});

test("appstate web-session route handle ratchet rejects production callback storage", () => {
  const violations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub(in crate::daemon) type CreateWebSessionFuture =
        Pin<Box<dyn Future<Output = Result<WebSessionInfo, WebSessionLaunchError>> + Send + 'static>>;
      pub(in crate::daemon) type CreateWebSessionEffect =
        Arc<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>;
      type WebSessionLauncher =
        Box<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>;

      pub struct WebSessionRouteHandle {
        web_sessions: Arc<WebSessionManager>,
        create_web_session: CreateWebSessionEffect,
        create_web_session_alias: WebSessionLauncher,
        create_web_session_direct:
          Arc<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>,
        create_web_session_fn: fn(WebSessionLaunchRequest) -> CreateWebSessionFuture,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("web-session route production callback effect alias remains"),
  );
  assert(
    violations.includes("web-session route capability stores production callback effect"),
  );
  assert(
    violations.includes("web-session route capability stores production callback shape"),
  );
  assert(
    violations.includes("web-session route production callback-shaped alias remains"),
  );
});

test("appstate web-session route handle ratchet rejects production launch enum backdoors", () => {
  const violations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      type WebSessionLauncher =
        Box<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>;

      enum WebSessionRouteLaunch {
        Host(WebSessionLaunchHost),
        Callback(Arc<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>),
        AliasCallback(WebSessionLauncher),
        Broad(FooDomainHandle),
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes(
      "web-session route launch enum stores production callback effect",
    ),
  );
  assert(
    violations.includes(
      "web-session route launch enum stores broad handle or daemon state",
    ),
  );
  assert(
    violations.includes("web-session route production callback-shaped alias remains"),
  );
  assert(
    violations.includes(
      "web-session route launch enum exposes unexpected production variant",
    ),
  );
});

test("appstate web-session route handle ratchet allows test-gated callback alias use", () => {
  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        type WebSessionLauncher =
          Box<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>;

        enum WebSessionRouteLaunch {
          Host(WebSessionLaunchHost),
          #[cfg(test)]
          Override(WebSessionLauncher),
        }

        pub struct WebSessionRouteHandle {
          web_sessions: Arc<WebSessionManager>,
          launch: WebSessionRouteLaunch,
          #[cfg(test)]
          override_launch: WebSessionLauncher,
        }
      `,
    }),
    [],
  );
});

test("appstate web-session launch ratchet rejects DaemonState and broad runtime seams", () => {
  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/web_sessions/launch/context.rs",
      contents: `
        use crate::daemon::{DaemonState, ProviderWorkspaceLaunchRuntime};

        async fn resolve(state: Arc<DaemonState>, runtime: ProviderWorkspaceLaunchRuntime) {}
      `,
    }).map((violation) => violation.name),
    [
      "web-session launch production helper mentions DaemonState",
      "web-session launch production helper mentions DaemonState",
      "web-session launch production helper mentions broad daemon or route handle",
      "web-session launch production helper mentions broad daemon or route handle",
    ],
  );

  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/web_sessions.rs",
      contents: `
        use crate::daemon::{DaemonState, ProviderWorkspaceLaunchRuntime};

        async fn prepare(state: Arc<DaemonState>, runtime: ProviderWorkspaceLaunchRuntime) {}
      `,
    }).map((violation) => violation.name),
    [
      "web-session worker prep mentions broad daemon state or provider workspace runtime",
      "web-session worker prep mentions broad daemon state or provider workspace runtime",
      "web-session worker prep mentions broad daemon state or provider workspace runtime",
      "web-session worker prep mentions broad daemon state or provider workspace runtime",
    ],
  );
});

test("appstate web-session launch ratchet rejects broad host fields", () => {
  const launchViolations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/web_sessions/launch.rs",
    contents: `
      pub(in crate::daemon) struct WebSessionLaunchHost {
        state: Arc<DaemonState>,
        route: WebSessionRouteHandle,
        runtime: ProviderWorkspaceLaunchRuntime,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    launchViolations.includes(
      "web-session launch production helper mentions DaemonState",
    ),
  );
  assert(
    launchViolations.includes(
      "web-session launch host stores broad daemon or route handle",
    ),
  );
  assert(
    launchViolations.includes(
      "web-session launch host exposes generic full-state escape hatch",
    ),
  );

  const workerViolations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/web_sessions.rs",
    contents: `
      pub(in crate::daemon) struct WebSessionWorkerRuntimeHost {
        daemon_url: String,
        auth_token: Option<String>,
        harness: Arc<HarnessRuntimeManager>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    workerViolations.includes(
      "web-session worker runtime host exposes generic full-state escape hatch",
    ),
  );
});

test("appstate web-session launch ratchet rejects host callback backdoors", () => {
  const violations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/web_sessions/launch.rs",
    contents: `
      type WebSessionCreateHook = Arc<dyn Fn(WebSessionLaunchRequest) -> WebSessionLaunchResult>;

      pub(in crate::daemon) struct WebSessionLaunchHost {
        callback: WebSessionCreateHook,
        raw_create: fn(WebSessionLaunchRequest) -> WebSessionLaunchResult,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes(
      "web-session launch host stores production callback shape",
    ),
  );
  assert(
    violations.includes(
      "web-session launch host stores production callback-shaped alias",
    ),
  );
});

test("appstate web-session launch ratchet rejects imported host callback aliases", () => {
  const violations = scanWebSessionRouteHandleRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/web_sessions/launch/host.rs",
    contents: `
      use self::hooks::WebSessionCreateHook;

      pub(in crate::daemon) struct WebSessionLaunchHost {
        global_store: WebSessionCreateHook,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        data_root: PathBuf,
        worker_runtime: WebSessionWorkerRuntimeHost,
        web_sessions: Arc<WebSessionManager>,
        callback: WebSessionCreateHook,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("web-session launch host exposes unexpected production field"),
  );
  assert(
    violations.includes(
      "web-session launch host field type differs from narrow contract",
    ),
  );
});

test("appstate web-session launch ratchet allows narrow host dependencies", () => {
  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/web_sessions/launch.rs",
      contents: `
        pub(in crate::daemon) struct WebSessionLaunchHost {
          global_store: Store,
          workspace_stores: ProtectedWorkspaceStoreLookup,
          data_root: PathBuf,
          worker_runtime: WebSessionWorkerRuntimeHost,
          web_sessions: Arc<WebSessionManager>,
        }
      `,
    }),
    [],
  );

  assert.deepEqual(
    scanWebSessionRouteHandleRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/web_sessions.rs",
      contents: `
        pub(in crate::daemon) struct WebSessionWorkerRuntimeHost {
          data_root: PathBuf,
          providers: Arc<ProviderRuntime>,
          ops_events: OpsEvents,
        }
      `,
    }),
    [],
  );
});

test("appstate provider account route ratchet rejects broad providers handle", () => {
  const violations = scanProviderAccountHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/providers/accounts/amp.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.amp_accounts_for_route().await;
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider account route extracts broad providers handle",
      "provider account route extracts broad providers handle",
    ],
  );

  assert.deepEqual(
    scanProviderAccountHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/providers/accounts/codex/usage.rs",
      contents: `
        use ctx_daemon::daemon::ProviderUsageHandle;
        async fn route(State(providers): State<ProviderUsageHandle>) {}
      `,
    }),
    [],
  );
});

test("appstate provider account daemon facade ratchet rejects full-state route seam", () => {
  assert.deepEqual(
    scanProviderAccountDaemonFacadeRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/providers/accounts/routes/handle.rs",
      contents: `
        use crate::daemon::ProvidersHandle;
        impl ProvidersHandle {
          pub async fn amp_accounts_for_route(&self) {}
        }
      `,
    }).map((violation) => violation.name),
    [
      "provider account daemon facade implemented on broad providers handle",
      "provider account daemon facade implemented on broad providers handle",
    ],
  );

  assert.deepEqual(
    scanProviderAccountDaemonFacadeRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/providers/accounts/routes/operations.rs",
      contents: `
        use crate::daemon::{providers::accounts, DaemonState};
        async fn amp_accounts_response(state: &Arc<DaemonState>) {}
      `,
    }).map((violation) => violation.name),
    [
      "provider account route operation accepts daemon state",
      "provider account route operation accepts daemon state",
    ],
  );
});

test("appstate provider runtime surface route ratchet rejects broad providers handle", () => {
  const violations = scanProviderRuntimeSurfaceHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/providers/status/routes.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.providers_statuses_for_route(query).await;
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider runtime surface route extracts broad providers handle",
      "provider runtime surface route extracts broad providers handle",
    ],
  );
});

test("appstate provider runtime surface daemon facade ratchet rejects broad providers handle", () => {
  const singletonViolations = scanProviderRuntimeSurfaceDaemonFacadeRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/providers/usage.rs",
      contents: `
        use crate::daemon::ProvidersHandle;
        impl ProvidersHandle {
          pub async fn provider_usage_for_route(&self) {}
        }
      `,
    }).map((violation) => violation.name);
  assert.deepEqual(
    singletonViolations,
    [
      "provider runtime surface daemon facade implemented on broad providers handle",
      "provider runtime surface daemon facade implemented on broad providers handle",
    ],
  );

  const groupedViolations = scanProviderRuntimeSurfaceDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/status.rs",
    contents: `
      use crate::daemon::{ProviderStatusHandle, ProvidersHandle};
      async fn helper(handle: ProvidersHandle) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    groupedViolations,
    [
      "provider runtime surface daemon facade implemented on broad providers handle",
      "provider runtime surface daemon facade implemented on broad providers handle",
    ],
  );
});

test("appstate provider harness config route ratchet rejects broad providers handle", () => {
  const violations = scanProviderHarnessConfigHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/providers/harness_config.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.get_provider_harness_config_for_route("codex").await;
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider harness config route extracts broad providers handle",
      "provider harness config route extracts broad providers handle",
    ],
  );

  assert.deepEqual(
    scanProviderHarnessConfigHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
      contents: `
        use ctx_daemon::daemon::ProviderHarnessConfigHandle;
        async fn route(State(providers): State<ProviderHarnessConfigHandle>) {}
      `,
    }),
    [],
  );
});

test("appstate provider harness config daemon facade ratchet rejects full-state route seam", () => {
  const broadHandleViolations = scanProviderHarnessConfigDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/harness_config/routes.rs",
    contents: `
      use crate::daemon::ProvidersHandle;
      impl ProvidersHandle {
        pub async fn get_provider_harness_config_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    broadHandleViolations,
    [
      "provider harness config daemon facade implemented on broad providers handle",
      "provider harness config daemon facade implemented on broad providers handle",
    ],
  );

  const daemonStateViolations = scanProviderHarnessConfigDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/harness_config/routes.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn get_config(state: &Arc<DaemonState>) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    daemonStateViolations,
    [
      "provider harness config daemon facade accepts daemon state",
      "provider harness config daemon facade accepts daemon state",
    ],
  );
});

test("appstate provider bootstrap route ratchet rejects broad providers handle", () => {
  const violations = scanProviderBootstrapHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/providers/bootstrap.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.workspace_providers_bootstrap_for_route(req).await;
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider bootstrap route extracts broad providers handle",
      "provider bootstrap route extracts broad providers handle",
    ],
  );

  assert.deepEqual(
    scanProviderBootstrapHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/providers/bootstrap.rs",
      contents: `
        use ctx_daemon::daemon::ProviderBootstrapHandle;
        async fn route(State(providers): State<ProviderBootstrapHandle>) {}
      `,
    }),
    [],
  );
});

test("appstate provider bootstrap daemon facade ratchet rejects full-state route seam", () => {
  const broadHandleViolations = scanProviderBootstrapDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/bootstrap.rs",
    contents: `
      use crate::daemon::ProvidersHandle;
      impl ProvidersHandle {
        pub async fn workspace_providers_bootstrap_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    broadHandleViolations,
    [
      "provider bootstrap daemon facade implemented on broad providers handle",
      "provider bootstrap daemon facade implemented on broad providers handle",
    ],
  );

  const daemonStateViolations = scanProviderBootstrapDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/bootstrap.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn bootstrap(state: &Arc<DaemonState>) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    daemonStateViolations,
    [
      "provider bootstrap daemon facade accepts daemon state",
      "provider bootstrap daemon facade accepts daemon state",
    ],
  );
});

test("appstate provider workspace launch route ratchet rejects broad providers handle", () => {
  const violations = scanProviderWorkspaceLaunchHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/options.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.get_provider_options_for_route(req).await;
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider workspace launch route extracts broad providers handle",
      "provider workspace launch route extracts broad providers handle",
    ],
  );

  assert.deepEqual(
    scanProviderWorkspaceLaunchHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/auth/verify.rs",
      contents: `
        use ctx_daemon::daemon::ProviderWorkspaceAuthHandle;
        async fn route(State(auth): State<ProviderWorkspaceAuthHandle>) {}
      `,
    }),
    [],
  );
});

test("appstate provider workspace launch daemon facade ratchet rejects full-state route seam", () => {
  const broadHandleViolations = scanProviderWorkspaceLaunchDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/options/provider_options.rs",
    contents: `
      use crate::daemon::ProvidersHandle;
      impl ProvidersHandle {
        pub async fn get_provider_options_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    broadHandleViolations,
    [
      "provider workspace launch daemon facade implemented on broad providers handle",
      "provider workspace launch daemon facade implemented on broad providers handle",
    ],
  );

  const daemonStateViolations = scanProviderWorkspaceLaunchDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/auth_check.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn verify(state: &Arc<DaemonState>) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    daemonStateViolations,
    [
      "provider workspace launch daemon facade accepts daemon state",
      "provider workspace launch daemon facade accepts daemon state",
    ],
  );
});

test("appstate provider install route ratchet rejects broad providers handle", () => {
  const violations = scanProviderInstallHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/installs/start.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.start_provider_install_for_route("codex", None).await;
      }
    `,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider install route extracts broad providers handle",
      "provider install route extracts broad providers handle",
    ],
  );

  assert.deepEqual(
    scanProviderInstallHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/installs/status.rs",
      contents: `
        use ctx_daemon::daemon::ProviderInstallHandle;
        async fn route(State(installs): State<ProviderInstallHandle>) {}
      `,
    }),
    [],
  );

  const adminViolations = scanProviderInstallHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/installs/start.rs",
    contents: `
      use ctx_daemon::daemon::ProviderAdminHandle;
      async fn route(State(admin): State<ProviderAdminHandle>) {
        admin.refresh_provider_matrix_for_route().await;
      }
    `,
  });
  assert.deepEqual(
    adminViolations.map((violation) => violation.name),
    [
      "provider install route extracts provider admin handle",
      "provider install route extracts provider admin handle",
    ],
  );
});

test("appstate provider install daemon facade ratchet rejects full-state route seam", () => {
  const broadHandleViolations = scanProviderInstallDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/installs.rs",
    contents: `
      use crate::daemon::ProvidersHandle;
      impl ProvidersHandle {
        pub async fn start_provider_install_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    broadHandleViolations,
    [
      "provider install daemon facade implemented on broad providers handle",
      "provider install daemon facade implemented on broad providers handle",
    ],
  );

  const adminHandleViolations = scanProviderInstallDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/installs.rs",
    contents: `
      use crate::daemon::ProviderAdminHandle;
      impl ProviderAdminHandle {
        pub async fn start_provider_install_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    adminHandleViolations,
    [
      "provider install daemon facade implemented on provider admin handle",
      "provider install daemon facade implemented on provider admin handle",
    ],
  );

  const daemonStateViolations = scanProviderInstallDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/installs.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn status(state: &Arc<DaemonState>) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    daemonStateViolations,
    [
      "provider install daemon facade accepts daemon state",
      "provider install daemon facade accepts daemon state",
    ],
  );
});

test("appstate provider auth import route ratchet rejects broad provider handles", () => {
  const broadViolations = scanProviderAuthImportHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/providers/imports.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.import_provider_auth_candidates_for_route(req).await;
      }
    `,
  });
  assert.deepEqual(
    broadViolations.map((violation) => violation.name),
    [
      "provider auth import route extracts broad providers handle",
      "provider auth import route extracts broad providers handle",
    ],
  );

  const adminViolations = scanProviderAuthImportHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/providers/imports.rs",
    contents: `
      use ctx_daemon::daemon::ProviderAdminHandle;
      async fn route(State(admin): State<ProviderAdminHandle>) {
        admin.refresh_provider_matrix_for_route().await;
      }
    `,
  });
  assert.deepEqual(
    adminViolations.map((violation) => violation.name),
    [
      "provider auth import route extracts provider admin handle",
      "provider auth import route extracts provider admin handle",
    ],
  );

  assert.deepEqual(
    scanProviderAuthImportHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/providers/imports.rs",
      contents: `
        use ctx_daemon::daemon::ProviderAuthImportHandle;
        async fn route(State(auth_import): State<ProviderAuthImportHandle>) {}
      `,
    }),
    [],
  );
});

test("appstate provider auth import daemon facade ratchet rejects full-state route seam", () => {
  const broadHandleViolations = scanProviderAuthImportDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/auth_import.rs",
    contents: `
      use crate::daemon::ProvidersHandle;
      impl ProvidersHandle {
        pub async fn import_provider_auth_candidates_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    broadHandleViolations,
    [
      "provider auth import daemon facade implemented on broad providers handle",
      "provider auth import daemon facade implemented on broad providers handle",
    ],
  );

  const adminHandleViolations = scanProviderAuthImportDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/auth_import.rs",
    contents: `
      use crate::daemon::ProviderAdminHandle;
      impl ProviderAdminHandle {
        pub async fn import_provider_auth_candidates_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    adminHandleViolations,
    [
      "provider auth import daemon facade implemented on provider admin handle",
      "provider auth import daemon facade implemented on provider admin handle",
    ],
  );

  const daemonStateViolations = scanProviderAuthImportDaemonFacadeRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/auth_import.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn import(state: &Arc<DaemonState>) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    daemonStateViolations,
    [
      "provider auth import daemon facade accepts daemon state",
      "provider auth import daemon facade accepts daemon state",
    ],
  );
});

test("appstate provider login route ratchet rejects broad providers handle", () => {
  const violations = scanProviderLoginHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/providers/login/kimi.rs",
    contents: `
      use ctx_daemon::daemon::ProvidersHandle;
      async fn route(State(providers): State<ProvidersHandle>) {
        providers.start_kimi_login_for_route(req).await;
      }
    `,
  });
  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider login route extracts broad providers handle",
      "provider login route extracts broad providers handle",
    ],
  );

  assert.deepEqual(
    scanProviderLoginHandleRatchet({
      filePath: "core/crates/ctx-http/src/api/providers/login/kimi.rs",
      contents: `
        use ctx_daemon::daemon::ProviderAccountsHandle;
        async fn route(State(accounts): State<ProviderAccountsHandle>) {
          accounts.start_kimi_login_for_route(req).await;
        }
      `,
    }),
    [],
  );
});

test("appstate provider login daemon implementation ratchet rejects full-state seam", () => {
  const broadHandleViolations = scanProviderLoginDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/login_routes.rs",
    contents: `
      use crate::daemon::ProvidersHandle;
      impl ProvidersHandle {
        pub async fn start_kimi_login_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    broadHandleViolations,
    [
      "provider login daemon implementation uses broad providers handle",
      "provider login daemon implementation uses broad providers handle",
    ],
  );

  const daemonStateViolations = scanProviderLoginDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/providers/kimi_oauth_login.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn start(state: &Arc<DaemonState>) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(
    daemonStateViolations,
    [
      "provider login daemon implementation accepts daemon state",
      "provider login daemon implementation accepts daemon state",
    ],
  );
});

test("appstate guard rejects task creation placeholder extractors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/tasks/creation_task.rs",
    contents: `
      async fn create_task(
        State(tasks): State<TasksHandle>,
        State(_sessions): State<SessionsHandle>,
        State(_providers): State<ProvidersHandle>,
        State(_workspaces): State<WorkspacesHandle>,
        State(_transport): State<TransportHandle>,
      ) {}
    `,
    patterns: TASK_CREATION_PLACEHOLDER_EXTRACTOR_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "task creation placeholder session extractor",
      "task creation placeholder provider extractor",
      "task creation placeholder workspace extractor",
      "task creation placeholder transport extractor",
    ],
  );
});

test("appstate guard rejects task admission broad route handles", () => {
  const violations = scanTaskAdmissionHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/tasks/creation_session/create.rs",
    contents: `
      async fn create_session_for_task(
        State(tasks): State<TasksHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, ["task admission route extracts broad tasks handle"]);
});

test("appstate guard rejects task admission broad daemon seams", () => {
  const daemonViolations = scanTaskAdmissionDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/tasks/create_session.rs",
    contents: `
      use crate::daemon::{DaemonHandle, TasksHandle, SessionsHandle, ProvidersHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;
      fn rebuild(handle: &TasksHandle, state: Arc<DaemonState>) {
        let daemon = DaemonHandle::new(handle.state.clone());
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(daemonViolations), new Set([
    "task admission daemon implementation uses broad daemon handle",
    "task admission daemon implementation uses broad task/session/provider/workspace handle",
    "task admission daemon implementation accepts daemon state",
  ]));

  const fieldViolations = scanTaskAdmissionHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct TaskCreationHandle {
        tasks: TasksHandle,
      }
      pub struct TaskSessionAdmissionHandle {
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(fieldViolations, [
    "task admission capability stores broad handle or daemon state",
    "task admission capability stores broad handle or daemon state",
  ]);
});

test("appstate guard rejects task lifecycle broad route handles", () => {
  const violations = scanTaskLifecycleHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/tasks/handlers/archive.rs",
    contents: `
      async fn archive_task(
        State(tasks): State<TasksHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, ["task lifecycle route extracts broad tasks handle"]);
});

test("appstate guard rejects task lifecycle broad daemon seams", () => {
  const daemonViolations = scanTaskLifecycleDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/tasks/lifecycle.rs",
    contents: `
      use crate::daemon::{DaemonHandle, TasksHandle, SessionsHandle, ProvidersHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;
      impl TasksHandle {
        fn archive(handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(daemonViolations), new Set([
    "task lifecycle daemon implementation uses broad daemon handle",
    "task lifecycle daemon implementation uses broad task/session/provider/workspace handle",
    "task lifecycle daemon implementation accepts daemon state",
  ]));

  const fieldViolations = scanTaskLifecycleHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct TaskLifecycleHandle {
        tasks: TasksHandle,
      }
      fn task_creation(&self) -> TaskCreationHandle {
        let delete_loaded_task_with_cleanup = Arc::new(move || {
          let tasks = TasksHandle::new(Arc::clone(&state));
        });
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(fieldViolations, [
    "task lifecycle capability stores broad handle or daemon state",
    "task creation cleanup reconstructs broad tasks handle",
  ]);
});

test("appstate guard rejects task read metadata broad route handles", () => {
  const handlerViolations = scanTaskReadMetadataHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/tasks/handlers/listing.rs",
    contents: `
      async fn list_task_sessions(
        State(tasks): State<TasksHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "task read metadata route extracts broad tasks handle",
  ]);

  const routerViolations = scanTaskReadMetadataHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      use ctx_daemon::daemon::TasksHandle;
      pub(in crate::api) tasks: TasksHandle,
      impl_route_state_extractors! {
        TasksHandle, tasks;
      }
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { tasks: handle.tasks() }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "task read metadata route exposes broad tasks handle",
  ]));
});

test("appstate guard rejects task read metadata broad daemon seams", () => {
  const daemonViolations = scanTaskReadMetadataDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/tasks/metadata.rs",
    contents: `
      use crate::daemon::{DaemonHandle, TasksHandle, SessionsHandle, ProvidersHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;
      impl TasksHandle {
        fn update_title(handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(daemonViolations), new Set([
    "task read metadata daemon implementation uses broad daemon handle",
    "task read metadata daemon implementation uses broad task/session/provider/workspace handle",
    "task read metadata daemon implementation accepts daemon state",
  ]));

  const fieldViolations = scanTaskReadMetadataHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct TaskReadStateHandle {
        tasks: TasksHandle,
      }
      pub struct TaskTitleHandle {
        state: Arc<DaemonState>,
      }
      fn rebuild(&self) {
        let tasks = TasksHandle::new(Arc::clone(&self.state));
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(fieldViolations, [
    "task read metadata capability stores broad handle or daemon state",
    "task read metadata capability stores broad handle or daemon state",
    "task read metadata reconstructs broad tasks handle",
  ]);
});

test("appstate guard scans moved task route handle definitions", () => {
  const filePath = "core/crates/ctx-daemon/src/daemon/task_route_handles.rs";
  const contents = `
    pub struct TaskCreationHandle {
      creation_tasks: TasksHandle,
    }

    pub struct TaskSessionAdmissionHandle {
      admission_state: Arc<DaemonState>,
    }

    pub struct TaskLifecycleHandle {
      lifecycle_daemon: DaemonHandle,
    }

    pub struct TaskListingHandle {
      listing_sessions: SessionsHandle,
    }

    pub struct TaskReadStateHandle {
      read_state_workspaces: WorkspacesHandle,
    }
  `;

  const admissionViolations = scanTaskAdmissionHandleFieldRatchet({
    filePath,
    contents,
  }).map((violation) => violation.text);

  assert.deepEqual(admissionViolations, [
    "creation_tasks: TasksHandle,",
    "admission_state: Arc<DaemonState>,",
  ]);

  const lifecycleViolations = scanTaskLifecycleHandleFieldRatchet({
    filePath,
    contents,
  }).map((violation) => violation.text);

  assert.deepEqual(lifecycleViolations, [
    "lifecycle_daemon: DaemonHandle,",
  ]);

  const readMetadataViolations = scanTaskReadMetadataHandleFieldRatchet({
    filePath,
    contents,
  }).map((violation) => violation.text);

  assert.deepEqual(readMetadataViolations, [
    "listing_sessions: SessionsHandle,",
    "read_state_workspaces: WorkspacesHandle,",
  ]);
});

test("appstate guard scans moved session route handle definitions", () => {
  const filePath = "core/crates/ctx-daemon/src/daemon/session_route_handles.rs";
  const contents = `
    pub struct SessionFileCompletionsHandle {
      workspaces: WorkspacesHandle,
    }

    pub struct SessionArtifactsHandle {
      sessions: SessionsHandle,
    }

    pub struct SessionReadModelsHandle {
      daemon: DaemonHandle,
    }

    pub struct SessionTitleModelModeHandle {
      state: Arc<DaemonState>,
    }

    pub struct SessionMessageCommandHandle {
      scheduler: SessionsHandle,
    }

    pub struct SessionSubagentReadHandle {
      daemon: DaemonHandle,
    }

    pub struct SessionSubagentMcpReadHandle {
      state: Arc<DaemonState>,
    }

    pub struct SessionVcsHandle {
      sessions: SessionsHandle,
    }
  `;

  const violations = [
    ...scanSessionControlHandleFieldRatchet({ filePath, contents }),
    ...scanSessionArtifactsHandleFieldRatchet({ filePath, contents }),
    ...scanSessionReadModelsHandleFieldRatchet({ filePath, contents }),
    ...scanSessionTitleModelModeHandleFieldRatchet({ filePath, contents }),
    ...scanSessionMessageCommandHandleFieldRatchet({ filePath, contents }),
    ...scanSessionSubagentReadHandleFieldRatchet({ filePath, contents }),
    ...scanSessionSubagentMcpReadHandleFieldRatchet({ filePath, contents }),
    ...scanSessionVcsHandleFieldRatchet({ filePath, contents }),
  ].map((violation) => violation.name);

  assert(violations.includes("session control capability stores broad handle or daemon state"));
  assert(violations.includes("session artifacts capability stores broad handle or daemon state"));
  assert(violations.includes("session read-model capability stores broad handle or daemon state"));
  assert(violations.includes("session title/model/mode capability stores broad handle or runtime bag"));
  assert(violations.includes("session message command capability stores broad handle or runtime bag"));
  assert(violations.includes("session subagent read capability stores broad handle or runtime bag"));
  assert(violations.includes("session subagent MCP read capability stores broad handle or runtime bag"));
  assert(violations.includes("session VCS capability stores broad handle or daemon state"));
});

test("appstate guard scans moved workspace stream route handle definitions", () => {
  const filePath = "core/crates/ctx-daemon/src/daemon/workspace_stream_route_handles.rs";
  const contents = `
    pub struct WorkspaceActiveHandle {
      state: Arc<DaemonState>,
    }

    pub(in crate::daemon) struct WorkspaceActiveHandleParts {
      daemon: DaemonHandle,
    }

    pub(in crate::daemon) struct WorkspaceActiveEffectsParts {
      with_state: Arc<dyn Fn() -> WorkspaceActiveFuture<()>>,
    }

    pub(in crate::daemon) struct WorkspaceActiveEffects {
      workspaces: WorkspacesHandle,
    }

    pub struct WorkspaceStreamHandle {
      sessions: SessionsHandle,
    }

    pub(in crate::daemon) struct WorkspaceStreamHandleParts {
      state: Arc<DaemonState>,
    }

    pub(in crate::daemon) struct WorkspaceStreamEffectsParts {
      with_daemon: Arc<dyn Fn() -> WorkspaceStreamFuture<()>>,
    }

    pub(in crate::daemon) struct WorkspaceStreamEffects {
      daemon: DaemonHandle,
    }

    pub struct WorkspaceVcsStreamHandle {
      daemon: DaemonHandle,
    }

    pub(in crate::daemon) struct WorkspaceVcsStreamHandleParts {
      workspace_runtime: WorkspaceRuntime,
    }

    pub struct WorkspaceVcsStreamRuntime {
      state: DaemonState,
    }

    pub(in crate::daemon) struct WorkspaceVcsStreamRuntimeParts {
      workspace_runtime: WorkspaceRuntime,
    }
  `;

  const violations = [
    ...scanWorkspaceActiveHandleFieldRatchet({ filePath, contents }),
    ...scanWorkspaceStreamHandleFieldRatchet({ filePath, contents }),
    ...scanWorkspaceVcsStreamHandleFieldRatchet({ filePath, contents }),
  ].map((violation) => violation.name);

  assert(violations.includes("workspace active capability stores broad handle or daemon state"));
  assert(violations.includes("workspace active effects stores broad handle or daemon state"));
  assert(
    violations.includes(
      "workspace active effects exposes generic full-state escape hatch",
    ),
  );
  assert(violations.includes("workspace stream capability stores broad handle or daemon state"));
  assert(violations.includes("workspace stream effects stores broad handle or daemon state"));
  assert(
    violations.includes(
      "workspace stream effects exposes generic full-state escape hatch",
    ),
  );
  assert(violations.includes("workspace VCS stream capability stores broad handle or daemon state"));
  assert(violations.includes("workspace VCS stream runtime stores broad handle or daemon state"));
});

test("appstate guard rejects session artifacts broad route handles", () => {
  const handlerViolations = scanSessionArtifactsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/artifacts/session/set.rs",
    contents: `
      async fn set_session_artifacts(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session artifacts route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionArtifactsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_artifacts: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionArtifactsHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session artifacts route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session artifacts broad daemon seams", () => {
  const daemonViolations = scanSessionArtifactsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/artifacts.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle};
      use crate::daemon::DaemonState;
      impl SessionArtifactsHandle {
        fn list(handle: DaemonHandle, sessions: SessionsHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(daemonViolations), new Set([
    "session artifacts daemon implementation uses broad daemon handle",
    "session artifacts daemon implementation uses broad session handle",
    "session artifacts daemon implementation accepts daemon state",
    "session artifacts daemon capability impl uses broad daemon handle",
    "session artifacts daemon capability impl uses broad session handle",
    "session artifacts daemon capability impl accepts daemon state",
  ]));

  const laterMethodViolations = scanSessionArtifactsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/artifact_access.rs",
    contents: `
      impl SessionArtifactsHandle {
        fn first(&self) {}

        fn second(&self, sessions: SessionsHandle, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(laterMethodViolations), new Set([
    "session artifacts daemon capability impl uses broad daemon handle",
    "session artifacts daemon capability impl uses broad session handle",
    "session artifacts daemon capability impl accepts daemon state",
  ]));

  const fieldViolations = scanSessionArtifactsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionArtifactsHandle {
        sessions: SessionsHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(fieldViolations, [
    "session artifacts capability stores broad handle or daemon state",
    "session artifacts capability stores broad handle or daemon state",
  ]);
});

test("appstate guard rejects session read-model broad route handles", () => {
  const handlerViolations = scanSessionReadModelsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn get_session_head(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session read-model route extracts broad sessions handle",
    "session read-model route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionReadModelsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_read_models: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionReadModelsHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session read-model route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session read-model broad daemon seams", () => {
  const daemonViolations = scanSessionReadModelsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/route_contract/read_models.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle};
      use crate::daemon::DaemonState;
      impl SessionsHandle {
        fn session_head_for_route(&self, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(daemonViolations), new Set([
    "session read-model daemon implementation uses broad daemon handle",
    "session read-model daemon implementation uses broad session handle",
    "session read-model daemon implementation accepts daemon state",
  ]));

  const capabilityViolations = scanSessionReadModelsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/read_models.rs",
    contents: `
      impl SessionReadModelsHandle {
        fn second(&self, sessions: SessionsHandle, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(capabilityViolations), new Set([
    "session read-model daemon implementation uses broad daemon handle",
    "session read-model daemon implementation uses broad session handle",
    "session read-model daemon implementation accepts daemon state",
    "session read-model capability impl uses broad daemon handle",
    "session read-model capability impl uses broad session handle",
    "session read-model capability impl accepts daemon state",
  ]));
});

test("appstate guard rejects session read-model broad handle fields", () => {
  const fieldViolations = scanSessionReadModelsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionReadModelsHandle {
        sessions: SessionsHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(fieldViolations, [
    "session read-model capability stores broad handle or daemon state",
    "session read-model capability stores broad handle or daemon state",
    "session read-model capability stores broad handle or daemon state",
    "session read-model capability exposes generic full-state field",
    "session read-model capability exposes generic full-state field",
    "session read-model capability exposes generic full-state field",
  ]);
});

test("appstate guard rejects session control broad route handles", () => {
  const handlerViolations = scanSessionControlHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/control/authenticate.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn authenticate_session(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session control route extracts broad sessions handle",
    "session control route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionControlHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_control: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionControlHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session control route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session file-completions broad route handles", () => {
  const handlerViolations = scanSessionFileCompletionsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/file_completions.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn session_file_completions(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session file-completions route extracts broad sessions handle",
    "session file-completions route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionFileCompletionsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_file_completions: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionFileCompletionsHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session file-completions route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session control broad daemon seams", () => {
  const daemonViolations = scanSessionControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/control_route.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle};
      use crate::daemon::DaemonState;
      impl SessionsHandle {
        fn cancel_session_for_route(&self, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(daemonViolations), new Set([
    "session control daemon implementation uses broad daemon handle",
    "session control daemon implementation uses broad session handle",
    "session control daemon implementation accepts daemon state",
  ]));

  const capabilityViolations = scanSessionControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/control_route.rs",
    contents: `
      impl SessionControlHandle {
        fn second(&self, sessions: SessionsHandle, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(capabilityViolations), new Set([
    "session control daemon implementation uses broad daemon handle",
    "session control daemon implementation uses broad session handle",
    "session control daemon implementation accepts daemon state",
    "session control capability impl uses broad daemon handle",
    "session control capability impl uses broad session handle",
    "session control capability impl accepts daemon state",
  ]));
});

test("appstate guard rejects hidden daemon-state session control assembly seams", () => {
  const violations = scanSessionControlAssemblyRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn session_control(&self) -> SessionControlHandle {
          let hidden = Arc::clone(&self.state);
          let cancel_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |session_id: SessionId, request_started: std::time::Instant| {
              let state = Arc::clone(&state);
              Box::pin(async move {
                crate::daemon::sessions::command_dispatch::cancel_session(&state, session_id).await?;
                crate::daemon::sessions::command_dispatch::interrupt_session(&state, session_id, request_started).await?;
                crate::daemon::sessions::auth::run_session_authentication(&state, &store, &session, None).await?;
                crate::daemon::sessions::ask_user::submit_ask_user_answer(&state, session_id, submission).await
              })
            }
          });
          todo!()
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session control assembly captures full daemon state",
    "session control assembly reuses full-state cancel helper",
    "session control assembly reuses full-state interrupt helper",
    "session control assembly reuses full-state auth helper",
    "session control assembly reuses full-state ask-user helper",
  ]));
});

test("appstate guard rejects session control broad handle fields", () => {
  const fieldViolations = scanSessionControlHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionControlHandle {
        sessions: SessionsHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
      pub struct SessionFileCompletionsEffects {
        workspaces: WorkspacesHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(fieldViolations, [
    "session control capability stores broad handle or daemon state",
    "session control capability stores broad handle or daemon state",
    "session control capability stores broad handle or daemon state",
    "session control capability exposes generic full-state field",
    "session control capability exposes generic full-state field",
    "session control capability exposes generic full-state field",
    "session control capability stores broad handle or daemon state",
    "session control capability exposes generic full-state field",
  ]);
});

test("appstate guard rejects session_control_effects mini-app-state escape hatches", () => {
  const violations = scanSessionControlHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/session_control_effects.rs",
    contents: `
      pub struct SessionControlHandleParts {
        daemon: DaemonHandle,
        services: SessionAuthHost,
        runtime: SessionControlRuntime,
        provider_services: ProviderServices,
      }
      pub struct SessionControlCommandHost {
        state: Arc<DaemonState>,
      }
      pub struct SessionAuthHost {
        state: Arc<DaemonState>,
      }
      pub struct SessionAuthRuntimeHost {
        state: Arc<DaemonState>,
      }
      pub struct SessionAuthEventHost {
        state: Arc<DaemonState>,
      }
      pub struct SessionAskUserHost {
        state: Arc<DaemonState>,
      }
      impl SessionAuthRuntimeHost {
        fn daemon_state(&self) -> &Arc<DaemonState> {
          &self.state
        }

        pub fn services(&self) -> &SessionAuthHost {
          todo!()
        }
      }
      impl SessionControlHandle {
        pub fn state(&self, state: SessionsHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session control capability stores broad handle or daemon state",
    "session control capability exposes generic full-state field",
    "session control capability method uses broad handle or daemon state",
    "session control capability exposes generic full-state method",
  ]));
});

test("appstate guard rejects auth runtime full-state helper backdoors", () => {
  const violations = scanSessionControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/auth/runtime.rs",
    contents: `
      use crate::daemon::DaemonState;

      pub(super) async fn prepare_session_auth_runtime(
        state: &Arc<DaemonState>,
      ) {
        let _ = resolve_existing_worktree_execution(state, store, &workspace, worktree_id).await;
        let _ = effective_install_target_for_environment(state.as_ref(), workspace_id, execution_environment).await;
        let _ = ensure_provider_adapter_for_target_with_cfg(state.as_ref(), &cfg, &provider_id, install_target).await;
        let _ = provider_auth_context_for_worktree_runtime(state.as_ref(), &worktree, &provider_id).await;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session control daemon implementation accepts daemon state",
    "session control auth runtime reuses full-state worktree execution helper",
    "session control auth runtime reuses full-state install-target helper",
    "session control auth runtime reuses full-state provider adapter helper",
    "session control auth runtime reuses full-state provider auth context helper",
  ]));
});

test("appstate guard rejects auth/event/ask-user daemon-state surfaces after migration", () => {
  const authViolations = scanSessionControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/auth.rs",
    contents: `
      use crate::daemon::DaemonState;

      pub async fn run_session_authentication(
        state: &Arc<DaemonState>,
      ) {
        let _ = prepare_session_auth_runtime(state, store, session).await;
        let _ = spawn_session_auth_event_sink(Arc::clone(state), store.clone(), session.id);
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(new Set(authViolations), new Set([
    "session control daemon implementation accepts daemon state",
    "session control auth implementation reuses full-state runtime prep helper",
    "session control auth implementation reuses full-state auth event sink",
  ]));

  const eventViolations = scanSessionControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/auth/events.rs",
    contents: `
      use crate::daemon::DaemonState;

      pub(super) fn spawn_session_auth_event_sink(state: Arc<DaemonState>) {}
      pub(super) async fn append_auth_notice(state: &Arc<DaemonState>) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(new Set(eventViolations), new Set([
    "session control daemon implementation accepts daemon state",
  ]));

  const askUserViolations = scanSessionControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/ask_user.rs",
    contents: `
      use crate::daemon::DaemonState;

      pub async fn submit_ask_user_answer(
        state: &Arc<DaemonState>,
      ) {}

      async fn store_for_ask_user_session(
        state: &Arc<DaemonState>,
      ) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(new Set(askUserViolations), new Set([
    "session control daemon implementation accepts daemon state",
  ]));
});

test("appstate guard rejects stale session file-completion broad helper", () => {
  const violations = scanSessionFileCompletionsDaemonSeamRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/workspace_bridge.rs",
    contents: `
      impl SessionsHandle {
        pub async fn complete_files_for_session(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "session file-completions broad session helper remains",
  ]);
});

test("appstate guard rejects hidden session file-completion daemon state seam", () => {
  const violations = scanSessionFileCompletionsDaemonSeamRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn session_file_completions(&self) -> SessionFileCompletionsHandle {
          let complete_files_for_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |session_id, query, limit| {
              complete_files_for_session(&state, session_id, query, limit)
            }
          });
          SessionFileCompletionsHandle::new(complete_files_for_session)
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session file-completions handle hides daemon state seam",
  ]));
});

test("appstate guard rejects session title/model/mode broad route handles", () => {
  const handlerViolations = scanSessionTitleModelModeHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/titles_and_modes/model.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn set_session_model(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session title/model/mode route extracts broad sessions handle",
    "session title/model/mode route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionTitleModelModeHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_title_model_mode: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionTitleModelModeHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session title/model/mode route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session title/model/mode broad daemon seams", () => {
  const violations = scanSessionTitleModelModeDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/title_model_mode_route.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle};
      use crate::daemon::DaemonState;
      impl SessionsHandle {
        fn set_session_model_for_route(&self, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session title/model/mode daemon implementation uses broad daemon handle",
    "session title/model/mode daemon implementation uses broad session handle",
    "session title/model/mode daemon implementation accepts daemon state",
  ]));
});

test("appstate guard rejects session title/model/mode broad handle fields", () => {
  const fieldViolations = scanSessionTitleModelModeHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionTitleModelModeHandle {
        sessions: SessionsHandle,
        workspaces: WorkspacesHandle,
        providers: ProvidersHandle,
        state: Arc<DaemonState>,
        store_manager: StoreManager,
        workspace_runtime: WorkspaceRuntime,
        effects: SessionTitleModelModeEffects,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(fieldViolations, [
    "session title/model/mode capability stores broad handle or runtime bag",
    "session title/model/mode capability stores broad handle or runtime bag",
    "session title/model/mode capability stores broad handle or runtime bag",
    "session title/model/mode capability stores broad handle or runtime bag",
    "session title/model/mode capability stores broad handle or runtime bag",
    "session title/model/mode capability stores broad handle or runtime bag",
    "session title/model/mode capability exposes generic full-state field",
    "session title/model/mode capability exposes generic full-state field",
    "session title/model/mode capability exposes generic full-state field",
    "session title/model/mode capability exposes generic full-state field",
    "session title/model/mode capability exposes generic full-state field",
  ]);
});

test("appstate guard rejects hidden session title/model/mode model-catalog daemon seam", () => {
  const violations = scanSessionTitleModelModeAssemblyRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn session_title_model_mode(&self) -> SessionTitleModelModeHandle {
          let model_catalog_loader = Arc::new({
            let state = Arc::clone(&self.state);
            move |workspace, provider_id, execution_environment| {
              load_provider_model_catalog_for_execution_environment(&state, &workspace, &provider_id, execution_environment)
            }
          });
          SessionTitleModelModeHandle::new(SessionTitleModelModeHandleParts {
            model_catalog_loader,
          })
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "session title/model/mode assembly hides daemon state model-catalog seam",
    "session title/model/mode assembly hides daemon state model-catalog seam",
    "session title/model/mode assembly hides daemon state model-catalog seam",
  ]);
});

test("appstate guard rejects broad seams in migrated title/model/mode title implementation", () => {
  const handleViolations = scanSessionTitleModelModeTitleImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/handle.rs",
    contents: `
      impl SessionTitleModelModeHandle {
        pub async fn generate_session_title_for_request(&self, state: Arc<DaemonState>, raw: DaemonState) {
          let sessions: SessionsHandle = todo!();
        }
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(handleViolations, [
    "session title/model/mode title implementation uses broad daemon seam",
    "session title/model/mode title implementation uses broad daemon seam",
    "session title/model/mode title implementation uses broad daemon seam",
  ]);

  const generationViolations = scanSessionTitleModelModeTitleImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/title_generation.rs",
    contents: `
      pub async fn maybe_generate_session_title_with_handle(
        handle: &SessionTitleModelModeHandle,
        state: Arc<DaemonState>,
      ) {
        let raw: DaemonState = todo!();
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(generationViolations, [
    "session title/model/mode title implementation uses broad daemon seam",
    "session title/model/mode title implementation uses broad daemon seam",
  ]);

  const persistenceViolations = scanSessionTitleModelModeTitleImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/title_generation/persistence.rs",
    contents: `
      pub async fn apply_session_title_update_with_handle(
        handle: &SessionTitleModelModeHandle,
        daemon: DaemonHandle,
      ) {}
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(persistenceViolations, [
    "session title/model/mode title implementation uses broad daemon seam",
  ]);
});

test("appstate guard rejects session message command broad route handles", () => {
  const handlerViolations = scanSessionMessageCommandHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/messages/post/handler.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn post_message(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session message command route extracts broad sessions handle",
    "session message command route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionMessageCommandHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_message_command: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionMessageCommandHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session message command route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session message command broad daemon seams", () => {
  const violations = scanSessionMessageCommandDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/message_commands.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle};
      use crate::daemon::DaemonState;
      impl SessionsHandle {
        fn post_user_message_for_request(&self, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session message command daemon implementation uses broad daemon handle",
    "session message command daemon implementation uses broad session handle",
    "session message command daemon implementation accepts daemon state",
  ]));
});

test("appstate guard covers split session message command helper files", () => {
  const attachmentViolations = scanSessionMessageCommandDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/message_attachment_signatures.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle};
      use crate::daemon::DaemonState;
      impl MessageAttachmentSignatureResolver for SessionMessageCommandHandle {
        fn bad(&self, handle: DaemonHandle, state: Arc<DaemonState>, sessions: SessionsHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(attachmentViolations), new Set([
    "session message command daemon implementation uses broad daemon handle",
    "session message command daemon implementation uses broad session handle",
    "session message command daemon implementation accepts daemon state",
  ]));

  const artifactViolations = scanSessionMessageCommandDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/artifact_access.rs",
    contents: `
      impl SessionsHandle {
        fn existing_artifact_route_impl_is_not_part_of_this_ratchet(&self) {}
      }

      impl SessionMessageCommandHandle {
        fn bad(&self, handle: DaemonHandle, state: Arc<DaemonState>, sessions: SessionsHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(artifactViolations), new Set([
    "session message command daemon implementation uses broad daemon handle",
    "session message command daemon implementation uses broad session handle",
    "session message command daemon implementation accepts daemon state",
  ]));
});

test("appstate guard rejects session message command broad handle fields", () => {
  const fieldViolations = scanSessionMessageCommandHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionMessageCommandHandle {
        sessions: SessionsHandle,
        workspaces: WorkspacesHandle,
        providers: ProvidersHandle,
        state: Arc<DaemonState>,
        store_manager: StoreManager,
        workspace_runtime: WorkspaceRuntime,
        effects: SessionMessageCommandEffects,
      }
      pub struct SessionMessageSchedulerSpawner {
        state: Arc<DaemonState>,
        weak_state: Weak<DaemonState>,
      }
      impl DaemonHandle {
        pub fn session_message_command(&self) -> SessionMessageCommandHandle {
          let state = Arc::clone(&self.state);
          let spawner = SessionMessageSchedulerSpawner::new(Arc::downgrade(&self.state));
          SessionMessageCommandHandle::new(state)
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(fieldViolations), new Set([
    "session message command capability stores broad handle or runtime bag",
    "session message command capability exposes generic full-state field",
    "session message scheduler spawner stores broad daemon state seam",
    "session message command assembly hides strong daemon state scheduler seam",
  ]));
});

test("appstate guard rejects scheduler host/runtime broad daemon-state seams", () => {
  const hostViolations = scanSchedulerHostStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/scheduler/host.rs",
    contents: `
      pub(in crate::daemon) struct SessionSchedulerWorkerHost {
        state: Arc<DaemonState>,
        state_weak: Weak<DaemonState>,
        event_loop_state: EventLoopState,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(hostViolations), new Set([
    "scheduler host/runtime surface depends on broad daemon state",
    "scheduler host/runtime surface exposes generic daemon-state field",
  ]));

  const runtimeViolations = scanSchedulerHostStateRatchet({
    filePath:
      "core/crates/ctx-daemon/src/daemon/scheduler/runtime/event_loop/provider_events.rs",
    contents: `
      use crate::daemon::DaemonState;
      pub(in crate::daemon::scheduler) struct TurnEventLoop {
        state_weak: Weak<DaemonState>,
      }
      async fn process(ctx: TurnEventLoopContext) {
        let state = ctx.state();
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(runtimeViolations), new Set([
    "scheduler host/runtime surface depends on broad daemon state",
    "scheduler host/runtime surface exposes generic daemon-state field",
  ]));

  const testFixtureViolations = scanSchedulerHostStateRatchet({
    filePath:
      "core/crates/ctx-daemon/src/daemon/scheduler/runtime/event_loop/tests/fixtures/setup.rs",
    contents: `
      pub struct Fixture {
        state: Arc<DaemonState>,
      }
    `,
  });

  assert.deepEqual(testFixtureViolations, []);
});

test("appstate guard rejects session subagent read broad route handles", () => {
  const handlerViolations = scanSessionSubagentReadHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/subagents/listings.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn list_session_subagents(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session subagent read route extracts broad sessions handle",
    "session subagent read route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionSubagentReadHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_subagent_read: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionSubagentReadHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session subagent read route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session subagent read broad daemon seams", () => {
  const violations = scanSessionSubagentReadDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/subagents_route.rs",
    contents: `
      impl SessionsHandle {
        pub async fn list_session_subagents_for_route(&self) {}
      }
      impl SessionSubagentReadHandle {
        fn bad(&self, handle: DaemonHandle, state: Arc<DaemonState>, sessions: SessionsHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session subagent read facade remains on broad sessions handle",
    "session subagent read daemon implementation uses broad seam",
  ]));
});

test("appstate guard rejects session subagent read broad fields and store escapes", () => {
  const violations = scanSessionSubagentReadHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionSubagentReadHandle {
        sessions: SessionsHandle,
        workspaces: WorkspacesHandle,
        providers: ProvidersHandle,
        state: Arc<DaemonState>,
        store_manager: StoreManager,
        workspace_runtime: WorkspaceRuntime,
        effects: SessionSubagentReadEffects,
      }
      impl SessionSubagentReadHandle {
        pub(in crate::daemon) async fn existing_session_store(&self, id: SessionId) -> Result<Store> {}
        pub(in crate::daemon) fn session_stores(&self) -> &SessionStoreLookup {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session subagent read capability stores broad handle or runtime bag",
    "session subagent read capability exposes generic full-state field",
    "session subagent read capability exposes store escape hatch",
  ]));
});

test("appstate guard rejects session subagent MCP read broad route handles", () => {
  const handlerViolations = scanSessionSubagentMcpReadHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/subagents/handlers.rs",
    contents: `
      async fn mcp_spawn_agent(State(sessions): State<SessionsHandle>) {}
      async fn mcp_list_agents(State(sessions): State<SessionsHandle>) {}
      async fn mcp_get_agent(State(state): State<SessionSubagentMcpReadHandle>) {}
      async fn mcp_wait_agent(State(state): State<SessionSubagentMcpReadHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(handlerViolations, [
    "session subagent MCP read route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionSubagentMcpReadHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_subagent_mcp_read: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionSubagentMcpReadHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session subagent MCP read route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session subagent MCP read broad daemon seams", () => {
  const routeViolations = scanSessionSubagentMcpReadDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/subagents_route.rs",
    contents: `
      impl SessionsHandle {
        pub async fn wait_agent_for_mcp_route(&self) {}
      }
      impl SessionSubagentMcpReadHandle {
        fn bad(&self, handle: DaemonHandle, state: Arc<DaemonState>, sessions: SessionsHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routeViolations), new Set([
    "session subagent MCP read facade remains on broad sessions handle",
    "session subagent MCP read daemon implementation uses broad seam",
  ]));

  const sessionsHandleViolations = scanSessionSubagentMcpReadDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/handle.rs",
    contents: `
      impl SessionsHandle {
        pub async fn list_agents(&self) {}
        pub async fn get_agent(&self) {}
        pub async fn wait_agent(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(sessionsHandleViolations), new Set([
    "session subagent MCP read request method remains on broad sessions handle",
  ]));

  const helperViolations = scanSessionSubagentMcpReadDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/subagents/details/builders.rs",
    contents: `
      async fn build_agent_detail_for_mcp_read(state: Arc<DaemonState>) {
        let _ = state.store_for_session(session_id).await;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(helperViolations), new Set([
    "session subagent MCP read helper uses broad daemon state",
  ]));
});

test("appstate guard rejects session subagent MCP read broad fields and store escapes", () => {
  const violations = scanSessionSubagentMcpReadHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionSubagentMcpReadHandle {
        sessions: SessionsHandle,
        state: Arc<DaemonState>,
        session_runtime: SessionRuntime<SchedulerCommand>,
        store_manager: StoreManager,
        effects: SessionSubagentMcpReadEffects,
      }
      impl SessionSubagentMcpReadHandle {
        pub(in crate::daemon) async fn existing_session_store(&self, id: SessionId) -> Result<Store> {}
        pub(in crate::daemon) fn public_store(&self) -> Store {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session subagent MCP read capability stores broad handle or runtime bag",
    "session subagent MCP read capability exposes generic full-state field",
    "session subagent MCP read capability exposes store escape hatch",
  ]));
});

test("appstate guard rejects session subagent MCP control broad route handles", () => {
  const initViolations = scanSessionSubagentMcpControlHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/subagents/init.rs",
    contents: `
      async fn mcp_spawn_agent(State(sessions): State<SessionsHandle>) {}
    `,
  }).map((violation) => violation.name);
  const handlerViolations = scanSessionSubagentMcpControlHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/subagents/handlers.rs",
    contents: `
      async fn mcp_send_input(State(sessions): State<SessionsHandle>) {}
      async fn mcp_archive_agent(State(state): State<SessionSubagentMcpControlHandle>) {}
      async fn mcp_interrupt_agent(State(sessions): State<SessionsHandle>) {}
      async fn mcp_wait_agent(State(sessions): State<SessionsHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(initViolations, [
    "session subagent MCP control route extracts broad sessions handle",
  ]);
  assert.deepEqual(handlerViolations, [
    "session subagent MCP control route extracts broad sessions handle",
    "session subagent MCP control route extracts broad sessions handle",
  ]);

  const routerViolations = scanSessionSubagentMcpControlHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_subagent_mcp_control: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionSubagentMcpControlHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session subagent MCP control route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session subagent MCP control broad daemon seams", () => {
  const routeViolations = scanSessionSubagentMcpControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/subagents_route.rs",
    contents: `
      impl SessionsHandle {
        pub async fn send_input_for_mcp_route(&self) {}
      }
      impl SessionSubagentMcpControlHandle {
        fn bad(&self, handle: DaemonHandle, state: Arc<DaemonState>, sessions: SessionsHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routeViolations), new Set([
    "session subagent MCP control facade remains on broad sessions handle",
    "session subagent MCP control daemon implementation uses broad seam",
  ]));

  const sessionsHandleViolations = scanSessionSubagentMcpControlDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/handle.rs",
    contents: `
      impl SessionsHandle {
        pub async fn spawn_agent(&self) {}
        pub async fn send_input(&self) {}
        pub async fn archive_agent(&self) {}
        pub async fn interrupt_agent(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(sessionsHandleViolations), new Set([
    "session subagent MCP control request method remains on broad sessions handle",
  ]));
});

test("appstate guard rejects session subagent MCP control broad fields and store escapes", () => {
  const violations = scanSessionSubagentMcpControlHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/subagents/mcp_control.rs",
    contents: `
      pub struct SessionSubagentMcpControlHandle {
        sessions: SessionsHandle,
        state: Arc<DaemonState>,
        store_manager: StoreManager,
        effects: SessionSubagentMcpControlEffects,
      }
      pub(in crate::daemon) struct SessionSubagentMcpControlHandleParts {
        workspaces: WorkspacesHandle,
      }
      impl SessionSubagentMcpControlHandle {
        pub(in crate::daemon) async fn existing_session_store(&self, id: SessionId) -> Result<Store> {}
        pub(in crate::daemon) fn public_store(&self) -> Store {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(violations), new Set([
    "session subagent MCP control capability stores broad handle or runtime bag",
    "session subagent MCP control capability exposes generic full-state field",
    "session subagent MCP control capability exposes store escape hatch",
  ]));
});

test("appstate guard rejects subagent spawn host daemon-state seams", () => {
  const spawnViolations = scanSubagentSpawnHostStateRatchet({
    filePath:
      "core/crates/ctx-daemon/src/daemon/sessions/subagents/agent_control/spawn.rs",
    contents: `
      pub(in crate::daemon) struct SubagentSpawnHost {
        daemon_state: Arc<DaemonState>,
        state: Arc<DaemonState>,
        effects: SubagentSpawnEffects,
      }
      pub(in crate::daemon) struct SubagentSpawnHostParts {
        weak: Weak<DaemonState>,
        sessions: SessionsHandle,
        task_workspace: Arc<TaskAdmissionWorkspaceRuntime>,
      }
      impl SubagentSpawnHost {
        pub(in crate::daemon) fn daemon_state(&self) -> Arc<DaemonState> {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(spawnViolations), new Set([
    "subagent spawn host stores broad daemon state seam",
    "subagent spawn host exposes generic full-state field",
    "subagent spawn host exposes daemon state escape hatch",
  ]));

  const childRunViolations = scanSubagentSpawnHostStateRatchet({
    filePath:
      "core/crates/ctx-daemon/src/daemon/sessions/subagents/child_runs/wait.rs",
    contents: `
      use crate::daemon::DaemonState;
      pub async fn wait_for_run_terminal_turn(state: &Weak<DaemonState>) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(childRunViolations), new Set([
    "subagent helper depends on broad daemon state",
  ]));

  const childRunStrongRuntimeViolations = scanSubagentSpawnHostStateRatchet({
    filePath:
      "core/crates/ctx-daemon/src/daemon/sessions/subagents/child_runs/invocation.rs",
    contents: `
      pub(in crate::daemon) struct SubagentChildRunHost {
        session_stores: SessionStoreLookup,
        publish_host: SessionSubagentMcpControlPublicationHost,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(childRunStrongRuntimeViolations), new Set([
    "subagent child-run host strongly retains session runtime",
  ]));

  const worktreeViolations = scanSubagentSpawnHostStateRatchet({
    filePath:
      "core/crates/ctx-daemon/src/daemon/sessions/subagents/agent_control/spawn/worktrees.rs",
    contents: `
      use crate::daemon::DaemonState;
      pub(in crate::daemon) struct SubagentSpawnWorktreePlanner {
        task_workspace: Arc<TaskAdmissionWorkspaceRuntime>,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(worktreeViolations), new Set([
    "subagent helper depends on broad daemon state",
  ]));

  const taskWorktreeHostViolations = scanSubagentSpawnHostStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/task_worktree_host.rs",
    contents: `
      use crate::daemon::{DaemonHandle, DaemonState};
      pub(in crate::daemon) struct SubagentSpawnWorktreeHost;
      pub(in crate::daemon) struct TaskWorktreeHost {
        state: Arc<DaemonState>,
        runtime: Arc<TaskLifecycleWorkspaceRuntime>,
      }
      pub(in crate::daemon) struct TaskWorktreeHostParts {
        effects: TaskAdmissionWorkspaceRuntime,
      }
      impl TaskWorktreeHost {
        fn make(&self) {
          let runtime = runtime_from_state();
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(taskWorktreeHostViolations), new Set([
    "task worktree host depends on broad daemon state",
    "task worktree host exposes generic full-state field",
  ]));

  const assemblyViolations = scanSubagentSpawnHostStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn session_subagent_mcp_control(&self) -> SessionSubagentMcpControlHandle {
          let hidden = Arc::clone(&self.state);
          let forbidden = SomeFactory::new(Arc::downgrade(&self.state));
          let task_workspace = self.task_session_admission_workspace_runtime();
          let scheduler_spawner =
            SessionSubagentMcpControlSchedulerSpawner::new(Arc::downgrade(&self.state));
          todo!()
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(assemblyViolations, [
    "task worktree host assembly keeps removed workspace runtime seam",
    "subagent spawn/control assembly hides daemon state capture",
    "subagent spawn/control assembly hides daemon state capture",
    "subagent spawn/control assembly hides daemon state capture",
    "subagent spawn/control assembly hides daemon state capture",
  ]);

  const taskWorktreeAssemblyViolations = scanSubagentSpawnHostStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        fn task_worktree_host(&self) -> Arc<TaskWorktreeHost> {
          let state = Arc::clone(&self.state);
          let weak = Arc::downgrade(&self.state);
          TaskWorktreeHost::new(state, weak)
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(taskWorktreeAssemblyViolations, [
    "task worktree host assembly captures broad daemon state",
    "task worktree host assembly captures broad daemon state",
  ]);
});

test("appstate guard rejects final session route broad HTTP state", () => {
  const titleViolations = scanFinalSessionRouteCapabilityRatchet({
    filePath: "core/crates/ctx-http/src/api/title_generation.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn get_title_generation_local_status(
        State(state): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(titleViolations, [
    "final session route extracts broad sessions handle",
    "final session route extracts broad sessions handle",
  ]);

  const demoViolations = scanFinalSessionRouteCapabilityRatchet({
    filePath: "core/crates/ctx-http/src/api/demo/seed_transcript.rs",
    contents: `
      async fn dev_seed_session_transcript(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(demoViolations, [
    "final session route extracts broad sessions handle",
  ]);

  const routerViolations = scanFinalSessionRouteCapabilityRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      pub struct RouteHandles {
        sessions: SessionsHandle,
      }
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { sessions: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionsHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "final session route exposes broad sessions handle in router state",
  ]));
});

test("appstate guard rejects final session route broad daemon seams", () => {
  const titleViolations = scanFinalSessionRouteCapabilityRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/title_generation.rs",
    contents: `
      impl SessionsHandle {
        pub async fn title_generation_local_status(&self) {}
        pub async fn start_title_generation_local_install(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(titleViolations, [
    "final session route facade remains on broad sessions handle",
    "final session route facade remains on broad sessions handle",
  ]);

  const demoViolations = scanFinalSessionRouteCapabilityRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/demo_seed.rs",
    contents: `
      impl SessionsHandle {
        pub async fn seed_demo_transcript(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(demoViolations, [
    "final session route facade remains on broad sessions handle",
  ]);
});

test("appstate guard rejects final session route capability broad fields and methods", () => {
  const titleViolations = scanFinalSessionRouteCapabilityRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/title_generation.rs",
    contents: `
      pub struct TitleGenerationLocalHandle {
        state: Arc<DaemonState>,
      }
      pub(in crate::daemon) struct TitleGenerationLocalInstallEffect {
        host: Arc<dyn ManagedInstallHost>,
      }
      impl TitleGenerationLocalHandle {
        fn bad(&self, handle: DaemonHandle, sessions: SessionsHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(titleViolations), new Set([
    "final session route capability stores broad daemon seam",
    "final session route capability exposes generic full-state field",
    "final session route capability method uses broad daemon seam",
  ]));

  const demoViolations = scanFinalSessionRouteCapabilityRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/demo_seed.rs",
    contents: `
      pub struct DemoSeedTranscriptHandle {
        sessions: SessionsHandle,
      }
      impl DemoSeedTranscriptHandle {
        fn bad(&self, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(demoViolations), new Set([
    "final session route capability stores broad daemon seam",
    "final session route capability exposes generic full-state field",
    "final session route capability method uses broad daemon seam",
  ]));
});

test("appstate guard rejects session VCS broad route handles", () => {
  const handlerViolations = scanSessionVcsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/sessions/snapshot/vcs/diff.rs",
    contents: `
      use ctx_daemon::daemon::SessionsHandle;
      async fn get_session_diff(
        State(sessions): State<SessionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(handlerViolations), new Set([
    "session VCS route extracts broad sessions handle",
  ]));

  const routerViolations = scanSessionVcsHandleRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self { session_vcs: handle.sessions() }
      }
      impl_route_state_extractors! {
        SessionVcsHandle, sessions;
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(routerViolations), new Set([
    "session VCS route exposes broad sessions handle",
  ]));
});

test("appstate guard rejects session VCS broad daemon seams", () => {
  const daemonViolations = scanSessionVcsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/vcs.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle};
      use crate::daemon::DaemonState;
      impl SessionVcsHandle {
        fn diff(handle: DaemonHandle, sessions: SessionsHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(new Set(daemonViolations), new Set([
    "session VCS daemon implementation uses broad daemon handle",
    "session VCS daemon implementation uses broad session handle",
    "session VCS daemon implementation accepts daemon state",
    "session VCS daemon capability impl uses broad daemon handle",
    "session VCS daemon capability impl uses broad session handle",
    "session VCS daemon capability impl accepts daemon state",
  ]));

  const laterMethodViolations = scanSessionVcsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/vcs_route.rs",
    contents: `
      impl SessionVcsHandle {
        fn first(&self) {}

        fn second(&self, sessions: SessionsHandle, handle: DaemonHandle, state: Arc<DaemonState>) {}
      }
    `,
  }).map((violation) => violation.name);

  assert(laterMethodViolations.includes("session VCS daemon capability impl uses broad daemon handle"));
  assert(laterMethodViolations.includes("session VCS daemon capability impl uses broad session handle"));
  assert(laterMethodViolations.includes("session VCS daemon capability impl accepts daemon state"));
});

test("appstate guard rejects session VCS broad handle and generic effects fields", () => {
  const fieldViolations = scanSessionVcsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionVcsHandle {
        sessions: SessionsHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }

      pub struct SessionVcsEffectsParts {
        with_state: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        with_daemon: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        daemon: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        state: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        broad_sessions: SessionsHandle,
        broad_daemon: DaemonHandle,
        broad_state: Arc<DaemonState>,
      }

      pub struct SessionVcsEffects {
        with_state: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        with_daemon: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        daemon: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        state: Arc<dyn Fn() -> SessionVcsFuture<()>>,
        broad_sessions: SessionsHandle,
        broad_daemon: DaemonHandle,
        broad_state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(fieldViolations.includes("session VCS capability stores broad handle or daemon state"));
  assert(fieldViolations.includes("session VCS effects stores broad handle or daemon state"));
  assert(fieldViolations.includes("session VCS effects exposes generic full-state escape hatch"));

  const narrowClosureViolations = scanSessionVcsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct SessionVcsEffects {
        load_git_status_snapshot: Arc<dyn Fn(SessionId) -> SessionVcsFuture<Result<()>> + Send + Sync>,
        apply_worktree_patch: Arc<dyn Fn(SessionId, String) -> SessionVcsFuture<Result<()>> + Send + Sync>,
      }
    `,
  });

  assert.deepEqual(narrowClosureViolations, []);
});

test("appstate guard rejects workspace stream route extraction outside entry routes", () => {
  const violations = scanWorkspaceStreamRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceStreamHandle;
      async fn handler(
        State(state): State<WorkspaceStreamHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace stream route extracts WorkspaceStreamHandle outside entry route",
  ]);

  for (const filePath of [
    "core/crates/ctx-http/src/api/ws/workspace_active.rs",
    "core/crates/ctx-http/src/api/ws/secure_mobile.rs",
  ]) {
    assert.deepEqual(
      scanWorkspaceStreamRouteExtractorRatchet({
        filePath,
        contents: `
          async fn handler(
            State(state): State<WorkspaceStreamHandle>,
          ) {}
        `,
      }),
      [],
    );
  }

  const routerViolations = scanWorkspaceStreamRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceStreamHandle;
      impl_route_state_extractors! {
        WorkspaceStreamHandle, workspace_stream;
      }
    `,
  });
  assert.deepEqual(routerViolations, []);
});

test("appstate guard rejects workspace stream active daemon broad seams", () => {
  const violations = scanWorkspaceStreamActiveDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/stream/read_model.rs",
    contents: `
      use crate::daemon::{DaemonHandle, SessionsHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspaceStreamHandle {
        async fn broad(
          &self,
          handle: DaemonHandle,
          sessions: SessionsHandle,
          workspaces: WorkspacesHandle,
          state: Arc<DaemonState>,
        ) {
          let _ = self.state.workspace_active_snapshot.clone();
          let _ = state.global_store().clone();
          let _ = state.store_for_session(session_id).await;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("workspace stream daemon implementation uses broad daemon state"));
  assert(violations.includes("workspace stream daemon implementation uses broad daemon handle"));
  assert(
    violations.includes(
      "workspace stream daemon implementation uses broad session/workspace handle",
    ),
  );
  assert(violations.includes("workspace stream daemon implementation uses broad state access"));

  const vcsViolations = scanWorkspaceStreamActiveDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/stream/vcs.rs",
    contents: `
      use crate::daemon::{DaemonState, WorkspacesHandle};
      async fn vcs_stream(state: Arc<DaemonState>, workspaces: WorkspacesHandle) {}
    `,
  });
  assert.deepEqual(vcsViolations, []);
});

test("appstate guard rejects workspace stream broad handle and generic effects fields", () => {
  const fieldViolations = scanWorkspaceStreamHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceStreamHandle {
        sessions: SessionsHandle,
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }

      pub(in crate::daemon) struct WorkspaceStreamHandleParts {
        with_state: Arc<dyn Fn() -> WorkspaceStreamFuture<()>>,
        sessions: SessionsHandle,
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }

      pub(in crate::daemon) struct WorkspaceStreamEffectsParts {
        with_state: Arc<dyn Fn() -> WorkspaceStreamFuture<()>>,
        with_daemon: Arc<dyn Fn() -> WorkspaceStreamFuture<()>>,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }

      pub(in crate::daemon) struct WorkspaceStreamEffects {
        with_state: Arc<dyn Fn() -> WorkspaceStreamFuture<()>>,
        with_daemon: Arc<dyn Fn() -> WorkspaceStreamFuture<()>>,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(fieldViolations.includes("workspace stream capability stores broad handle or daemon state"));
  assert(
    fieldViolations.includes(
      "workspace stream capability exposes generic full-state escape hatch",
    ),
  );
  assert(fieldViolations.includes("workspace stream effects stores broad handle or daemon state"));
  assert(
    fieldViolations.includes(
      "workspace stream effects exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceStreamHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceStreamHandle {
        global_store: Store,
        active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
        effects: Arc<WorkspaceStreamEffects>,
      }

      pub(in crate::daemon) struct WorkspaceStreamHandleParts {
        global_store: Store,
        active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
        effects: Arc<WorkspaceStreamEffects>,
      }

      pub(in crate::daemon) struct WorkspaceStreamEffectsParts {
        ensure_workspace_active_snapshot_hydrated: WorkspaceStreamHydrationEffect,
      }

      pub(in crate::daemon) struct WorkspaceStreamEffects {
        ensure_workspace_active_snapshot_hydrated: WorkspaceStreamHydrationEffect,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace active route extraction outside active route", () => {
  const violations = scanWorkspaceActiveRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceActiveHandle;
      async fn handler(
        State(state): State<WorkspaceActiveHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace active route extracts WorkspaceActiveHandle outside active route",
  ]);

  assert.deepEqual(
    scanWorkspaceActiveRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/active.rs",
      contents: `
        async fn handler(
          State(state): State<WorkspaceActiveHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace active broad route and router composition", () => {
  const routeViolations = scanWorkspaceActiveRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/active.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      async fn handler(State(state): State<WorkspacesHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert(routeViolations.includes("workspace active route uses broad workspace handle"));

  const routerViolations = scanWorkspaceActiveRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_active: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace active router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace active daemon facade broad seams", () => {
  const violations = scanWorkspaceActiveDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/active.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("workspace active daemon facade uses broad daemon state"));
  assert(violations.includes("workspace active daemon facade uses broad daemon handle"));
  assert(violations.includes("workspace active daemon facade uses broad workspace handle"));
});

test("appstate guard rejects workspace active broad handle fields and escape hatches", () => {
  const fieldViolations = scanWorkspaceActiveHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceActiveHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }

      pub(in crate::daemon) struct WorkspaceActiveHandleParts {
        with_state: Arc<dyn Fn() -> WorkspaceActiveFuture<()>>,
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }

      pub(in crate::daemon) struct WorkspaceActiveEffectsParts {
        with_state: Arc<dyn Fn() -> WorkspaceActiveFuture<()>>,
        with_daemon: Arc<dyn Fn() -> WorkspaceActiveFuture<()>>,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }

      pub(in crate::daemon) struct WorkspaceActiveEffects {
        with_state: Arc<dyn Fn() -> WorkspaceActiveFuture<()>>,
        with_daemon: Arc<dyn Fn() -> WorkspaceActiveFuture<()>>,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(fieldViolations.includes("workspace active capability stores broad handle or daemon state"));
  assert(
    fieldViolations.includes(
      "workspace active capability exposes generic full-state escape hatch",
    ),
  );
  assert(fieldViolations.includes("workspace active effects stores broad handle or daemon state"));
  assert(
    fieldViolations.includes(
      "workspace active effects exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceActiveHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceActiveHandle {
        active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
        effects: Arc<WorkspaceActiveEffects>,
      }

      pub(in crate::daemon) struct WorkspaceActiveHandleParts {
        active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
        effects: Arc<WorkspaceActiveEffects>,
      }

      pub(in crate::daemon) struct WorkspaceActiveEffectsParts {
        ensure_workspace_active_snapshot_hydrated: WorkspaceActiveHydrationEffect,
      }

      pub(in crate::daemon) struct WorkspaceActiveEffects {
        ensure_workspace_active_snapshot_hydrated: WorkspaceActiveHydrationEffect,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace active assembly through broad active loader", () => {
  const violations = scanWorkspaceActiveAssemblyRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn workspace_active(&self) -> WorkspaceActiveHandle {
          let state = Arc::clone(&self.state);
          let hydrate = Arc::new({
            let state = Arc::clone(&self.state);
            move |workspace_id| {
              let state = Arc::clone(&state);
              Box::pin(async move {
                state.ensure_workspace_active_snapshot_hydrated(workspace_id).await
              })
            }
          });
          let workspaces = self.workspaces();
          let load = workspaces.load_workspace_active_snapshot(workspace_id);
          WorkspaceActiveHandle::new(parts)
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("workspace active assembly captures full daemon state"));
  assert(violations.includes("workspace active assembly uses state-capturing closure"));
  assert(violations.includes("workspace active assembly uses broad workspace handle"));
  assert(violations.includes("workspace active assembly uses old broad active loader"));
});

test("appstate guard rejects workspace stream hidden full-state assembly", () => {
  const violations = scanWorkspaceStreamAssemblyRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn workspace_stream(&self) -> WorkspaceStreamHandle {
          let hydrate = Arc::new({
            let state = Arc::clone(&self.state);
            move |workspace_id| {
              let state = Arc::clone(&state);
              Box::pin(async move {
                state.ensure_workspace_active_snapshot_hydrated(workspace_id).await
              })
            }
          });
          let workspaces = self.workspaces();
          let load = workspaces.load_workspace_active_heads(workspace_id);
          WorkspaceStreamHandle::new(parts)
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("workspace stream assembly captures full daemon state"));
  assert(violations.includes("workspace stream assembly uses state-capturing closure"));
  assert(violations.includes("workspace stream assembly uses broad workspace handle"));
  assert(violations.includes("workspace stream assembly uses old broad active loader"));
});

test("appstate guard rejects worktree VCS hidden full-state assembly", () => {
  const violations = scanWorktreeVcsAssemblyRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn workspace_primary_branch(&self) -> WorkspacePrimaryBranchHandle {
          let refresh = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree| {
              let state = Arc::clone(&state);
              Box::pin(async move {
                state.refresh_worktree_vcs_snapshot(worktree).await
              })
            }
          });
          WorkspacePrimaryBranchHandle::new(refresh)
        }

        fn session_vcs_effects(&self) -> Arc<SessionVcsEffects> {
          let state = Arc::clone(&self.state);
          let load = Arc::new(move |worktree| {
            let state = Arc::clone(&state);
            Box::pin(async move { state.get_worktree_vcs_snapshot(worktree.id).await })
          });
          SessionVcsEffects::new(load)
        }

        pub fn workspace_vcs_stream(&self) -> WorkspaceVcsStreamHandle {
          let state = Arc::clone(&self.state);
          let refresh = Arc::new(move |worktree| {
            let state = Arc::clone(&state);
            Box::pin(async move { state.ensure_git_status_watcher(worktree).await })
          });
          WorkspaceVcsStreamHandle::new(refresh)
        }

        pub(in crate::daemon) fn workspace_vcs_stream_with_refresh_effect(
          &self,
          refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
        ) -> WorkspaceVcsStreamHandle {
          let state = Arc::clone(&self.state);
          let watch = Arc::new(move |worktree| {
            let state = Arc::clone(&state);
            Box::pin(async move { state.ensure_git_status_watcher(worktree).await })
          });
          WorkspaceVcsStreamHandle::new(watch, refresh_worktree_vcs)
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("worktree VCS assembly captures full daemon state"));
  assert(violations.includes("worktree VCS assembly uses state-capturing closure"));
  assert.equal(
    violations.filter(
      (name) => name === "worktree VCS assembly captures full daemon state",
    ).length,
    4,
  );
});

test("appstate guard accepts explicit worktree VCS assembly hosts", () => {
  const violations = scanWorktreeVcsAssemblyRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        pub fn workspace_primary_branch(&self) -> WorkspacePrimaryBranchHandle {
          let vcs_runtime = self.worktree_vcs_runtime_host();
          let vcs_execution = self.worktree_vcs_execution_host();
          let refresh = Arc::new(move |worktree| {
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            Box::pin(async move {
              emit_worktree_vcs_snapshot_for_worktree(&vcs_runtime, &vcs_execution, &worktree, true).await
            })
          });
          WorkspacePrimaryBranchHandle::new(refresh)
        }

        fn session_vcs_effects(&self) -> Arc<SessionVcsEffects> {
          let vcs_runtime = self.worktree_vcs_runtime_host();
          let vcs_execution = self.worktree_vcs_execution_host();
          let load = Arc::new(move |worktree_id| {
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            Box::pin(async move { vcs_runtime.get_worktree_vcs_snapshot(&vcs_execution, worktree_id).await })
          });
          SessionVcsEffects::new(load)
        }

        pub fn workspace_vcs_stream(&self) -> WorkspaceVcsStreamHandle {
          let vcs_runtime = self.worktree_vcs_runtime_host();
          let vcs_execution = self.worktree_vcs_execution_host();
          let refresh = Arc::new(move |worktree| {
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            Box::pin(async move {
              vcs_runtime.ensure_git_status_watcher(vcs_execution, worktree).await
            })
          });
          WorkspaceVcsStreamHandle::new(refresh)
        }

        pub(in crate::daemon) fn workspace_vcs_stream_with_refresh_effect(
          &self,
          refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
        ) -> WorkspaceVcsStreamHandle {
          let vcs_runtime = self.worktree_vcs_runtime_host();
          let vcs_execution = self.worktree_vcs_execution_host();
          let watch = Arc::new(move |worktree| {
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            Box::pin(async move {
              vcs_runtime.ensure_git_status_watcher(vcs_execution, worktree).await
            })
          });
          WorkspaceVcsStreamHandle::new(watch, refresh_worktree_vcs)
        }
      }
    `,
  });

  assert.deepEqual(violations, []);
});

test("appstate guard rejects broad daemon state in migrated worktree VCS helpers", () => {
  const violations = scanWorktreeVcsExplicitHostImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/git_status/source.rs",
    contents: `
      use std::sync::Arc;
      use crate::daemon::state::DaemonState;
      pub struct HttpWorktreeVcsSource {
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "worktree VCS explicit host implementation uses broad daemon state",
    "worktree VCS explicit host implementation uses broad daemon state",
  ]);
  assert.deepEqual(
    scanWorktreeVcsExplicitHostImplementationRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/git_status/source.rs",
      contents: `
        pub(in crate::daemon) struct HttpWorktreeVcsSource<'a> {
          execution: &'a WorktreeVcsExecutionHost,
        }
      `,
    }),
    [],
  );
});

test("appstate guard rejects resource utilization route extraction outside resource route", () => {
  const violations = scanResourceUtilizationRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::ResourceUtilizationHandle;
      async fn handler(
        State(state): State<ResourceUtilizationHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "resource utilization route extracts ResourceUtilizationHandle outside resource route",
  ]);

  assert.deepEqual(
    scanResourceUtilizationRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/resource_utilization.rs",
      contents: `
        async fn handler(
          State(state): State<ResourceUtilizationHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects resource utilization broad route and router composition", () => {
  const routeViolations = scanResourceUtilizationRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/resource_utilization.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      async fn handler(State(state): State<WorkspacesHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert(routeViolations.includes("resource utilization route uses broad workspace handle"));

  const routerViolations = scanResourceUtilizationRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            resource_utilization: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "resource utilization router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects resource utilization daemon facade broad seams", () => {
  const violations = scanResourceUtilizationDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/resource_utilization.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("resource utilization daemon facade uses broad daemon state"));
  assert(violations.includes("resource utilization daemon facade uses broad daemon handle"));
  assert(violations.includes("resource utilization daemon facade uses broad workspace handle"));
});

test("appstate guard rejects resource utilization broad handle fields", () => {
  const fieldViolations = scanResourceUtilizationHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct ResourceUtilizationHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "resource utilization capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "resource utilization capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanResourceUtilizationHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct ResourceUtilizationHandle {
        workspace_stores: ProtectedWorkspaceStoreLookup,
        providers: Arc<ProviderRuntime>,
        resource_sampler: Arc<Mutex<ResourceSampler>>,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects repo onboarding route extraction outside repo routes", () => {
  const violations = scanRepoOnboardingRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::RepoOnboardingHandle;
      async fn handler(
        State(state): State<RepoOnboardingHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "repo onboarding route extracts RepoOnboardingHandle outside repo routes",
  ]);

  for (const filePath of [
    "core/crates/ctx-http/src/api/repo/clone.rs",
    "core/crates/ctx-http/src/api/repo/destination.rs",
    "core/crates/ctx-http/src/api/repo/init.rs",
    "core/crates/ctx-http/src/api/repo/status.rs",
  ]) {
    assert.deepEqual(
      scanRepoOnboardingRouteExtractorRatchet({
        filePath,
        contents: `
          async fn handler(
            State(state): State<RepoOnboardingHandle>,
          ) {}
        `,
      }),
      [],
    );
  }
});

test("appstate guard rejects repo onboarding broad route and router composition", () => {
  const routeViolations = scanRepoOnboardingRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/repo/clone.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      async fn handler(State(state): State<WorkspacesHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert(routeViolations.includes("repo onboarding route uses broad workspace handle"));

  const routerViolations = scanRepoOnboardingRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            repo_onboarding: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, ["repo onboarding router composed from broad workspace handle"]);
});

test("appstate guard rejects repo onboarding daemon facade broad seams", () => {
  const violations = scanRepoOnboardingDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/repo_onboarding.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("repo onboarding daemon facade uses broad daemon state"));
  assert(violations.includes("repo onboarding daemon facade uses broad daemon handle"));
  assert(violations.includes("repo onboarding daemon facade uses broad workspace handle"));
});

test("appstate guard rejects repo onboarding broad handle fields", () => {
  const fieldViolations = scanRepoOnboardingHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/route_handles.rs",
    contents: `
      pub struct RepoOnboardingHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes("repo onboarding capability stores broad handle or daemon state"),
  );
  assert(
    fieldViolations.includes(
      "repo onboarding capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanRepoOnboardingHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/route_handles.rs",
    contents: `
      pub struct RepoOnboardingHandle {
        data_root: PathBuf,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard scans moved repo onboarding handle definitions in live path", () => {
  const violations = scanRepoOnboardingHandleDefinitionFiles({
    readFileForRelativePath(relativePath) {
      if (relativePath !== "core/crates/ctx-daemon/src/daemon/route_handles.rs") {
        return null;
      }
      return `
        pub struct RepoOnboardingHandle {
          daemon: DaemonHandle,
        }
      `;
    },
  });

  assert.deepEqual(
    violations.map((violation) => [violation.filePath, violation.name]),
    [
      [
        "core/crates/ctx-daemon/src/daemon/route_handles.rs",
        "repo onboarding capability stores broad handle or daemon state",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/route_handles.rs",
        "repo onboarding capability exposes generic full-state escape hatch",
      ],
    ],
  );
});

test("appstate guard detects workspace deletion capability in moved workspace route handle file", () => {
  assert.equal(
    workspaceDeletionCapabilityPresent({
      readFileForRelativePath(relativePath) {
        if (
          relativePath !==
          "core/crates/ctx-daemon/src/daemon/workspace_route_handles.rs"
        ) {
          return null;
        }
        return "pub struct WorkspaceDeletionHandle {}";
      },
    }),
    true,
  );
});

test("appstate guard scans moved workspace route handle definitions in live path", () => {
  const violations = scanWorkspaceRouteHandleDefinitionFiles({
    readFileForRelativePath(relativePath) {
      if (
        relativePath !==
        "core/crates/ctx-daemon/src/daemon/workspace_route_handles.rs"
      ) {
        return null;
      }
      return `
        pub struct WorkspaceDeletionHandle {
          daemon: DaemonHandle,
        }

        pub struct WorkspaceRegistryHandle {
          state: Arc<DaemonState>,
        }
      `;
    },
  });

  assert.deepEqual(
    violations.map((violation) => [violation.filePath, violation.name]),
    [
      [
        "core/crates/ctx-daemon/src/daemon/workspace_route_handles.rs",
        "workspace registry capability stores broad handle or daemon state",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/workspace_route_handles.rs",
        "workspace registry capability exposes generic full-state escape hatch",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/workspace_route_handles.rs",
        "workspace deletion capability stores broad handle or daemon state",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/workspace_route_handles.rs",
        "workspace deletion capability exposes generic full-state escape hatch",
      ],
    ],
  );
});

test("appstate guard scans moved provider route handle definitions in live path", () => {
  const violations = scanProviderRouteHandleDefinitionFiles({
    readFileForRelativePath(relativePath) {
      if (
        relativePath !==
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs"
      ) {
        return null;
      }
      return `
        pub struct ProviderStatusHandle {
          // A misleading } in a comment must not terminate the scanned struct.
          #[doc = "A misleading } in an attribute string must not terminate the scanned struct."]
          #[cfg_attr(any(), doc = br#"A misleading } in a raw byte string must not terminate the scanned struct."#)]
          #[doc = '}']
          workspace_stores: ProtectedWorkspaceStoreLookup,
          daemon: DaemonHandle,
          metadata: HashMap<
            String,
            String,
          >, // unexpected generic field
        }

        pub struct ProviderInstallHandle {
          state: Arc<DaemonState>, // unexpected field with trailing comment
        }

        pub struct ProviderUsageHandle {
          metadata: HashMap<String, String>
        }
      `;
    },
  });

  assert.deepEqual(
    violations.map((violation) => [violation.filePath, violation.name]),
    [
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider status capability declares unexpected field",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider status capability declares unexpected field",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider status capability declares unexpected field",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider status capability stores broad handle, store, or daemon state",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider status capability exposes generic full-state escape hatch",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider install capability declares unexpected field",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider install capability stores broad handle, store, or daemon state",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider install capability exposes generic full-state escape hatch",
      ],
      [
        "core/crates/ctx-daemon/src/daemon/provider_route_handles.rs",
        "provider usage capability declares unexpected field",
      ],
    ],
  );
});

test("appstate guard rejects run archive route extraction outside run archive route", () => {
  const violations = scanRunArchiveRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::RunArchiveHandle;
      async fn handler(
        State(state): State<RunArchiveHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "run archive route extracts RunArchiveHandle outside run archive route",
  ]);

  assert.deepEqual(
    scanRunArchiveRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/run_archive.rs",
      contents: `
        async fn handler(
          State(state): State<RunArchiveHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects run archive broad route and router composition", () => {
  const routeViolations = scanRunArchiveRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/run_archive.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      async fn handler(State(state): State<WorkspacesHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert(routeViolations.includes("run archive route uses broad workspace handle"));

  const routerViolations = scanRunArchiveRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            run_archive: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, ["run archive router composed from broad workspace handle"]);
});

test("appstate guard rejects run archive daemon facade broad seams", () => {
  const violations = scanRunArchiveDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/run_archive/ingest.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("run archive daemon facade uses broad daemon state"));
  assert(violations.includes("run archive daemon facade uses broad daemon handle"));
  assert(violations.includes("run archive daemon facade uses broad workspace handle"));
});

test("appstate guard rejects run archive broad handle fields", () => {
  const fieldViolations = scanRunArchiveHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct RunArchiveHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(fieldViolations.includes("run archive capability stores broad handle or daemon state"));
  assert(
    fieldViolations.includes("run archive capability exposes generic full-state escape hatch"),
  );

  const narrowViolations = scanRunArchiveHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct RunArchiveHandle {
        workspace_stores: ProtectedWorkspaceStoreLookup,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace org policy route extraction outside overlay route", () => {
  const violations = scanWorkspaceOrgPolicyRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/org_policy/enrollments.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceOrgPolicyHandle;
      async fn handler(
        State(state): State<WorkspaceOrgPolicyHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace org policy route extracts WorkspaceOrgPolicyHandle outside workspace overlay route",
  ]);

  assert.deepEqual(
    scanWorkspaceOrgPolicyRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/org_policy/workspace_overlay.rs",
      contents: `
        async fn handler(
          State(state): State<WorkspaceOrgPolicyHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace org policy broad route and router composition", () => {
  const routeViolations = scanWorkspaceOrgPolicyRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/org_policy/workspace_overlay.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      async fn handler(State(state): State<WorkspacesHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert(routeViolations.includes("workspace org policy route uses broad workspace handle"));

  const routerViolations = scanWorkspaceOrgPolicyRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_org_policy: handle.org_policy(),
            workspace_org_policy: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace org policy router composed from broad workspace/org-policy handle",
    "workspace org policy router composed from broad workspace/org-policy handle",
  ]);
});

test("appstate guard rejects workspace org policy daemon facade broad seams", () => {
  const violations = scanWorkspaceOrgPolicyDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/org_policy_route.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("workspace org policy daemon facade uses broad daemon state"));
  assert(violations.includes("workspace org policy daemon facade uses broad daemon handle"));
  assert(violations.includes("workspace org policy daemon facade uses broad workspace handle"));
});

test("appstate guard rejects workspace org policy broad handle fields", () => {
  const fieldViolations = scanWorkspaceOrgPolicyHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceOrgPolicyHandle {
        org_policy: OrgPolicyHandle,
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace org policy capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace org policy capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceOrgPolicyHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceOrgPolicyHandle {
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace prompt/bootstrap extraction outside allowed routes", () => {
  const violations = scanWorkspacePromptBootstrapConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacePromptBootstrapConfigHandle;
      async fn handler(
        State(state): State<WorkspacePromptBootstrapConfigHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace prompt/bootstrap config route extracts WorkspacePromptBootstrapConfigHandle outside prompt/bootstrap routes",
  ]);

  for (const filePath of [
    "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
    "core/crates/ctx-http/src/api/workspaces/management/prompt_config/agent.rs",
    "core/crates/ctx-http/src/api/workspaces/management/prompt_config/subagent.rs",
  ]) {
    assert.deepEqual(
      scanWorkspacePromptBootstrapConfigRouteExtractorRatchet({
        filePath,
        contents: `
          async fn handler(
            State(state): State<WorkspacePromptBootstrapConfigHandle>,
          ) {}
        `,
      }),
      [],
    );
  }
});

test("appstate guard rejects workspace prompt/bootstrap broad route and router composition", () => {
  const routeViolations = scanWorkspacePromptBootstrapConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      async fn handler(State(state): State<WorkspacesHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert(
    routeViolations.includes(
      "workspace prompt/bootstrap config route uses broad workspace handle",
    ),
  );

  const routerViolations = scanWorkspacePromptBootstrapConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_prompt_bootstrap_config: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace prompt/bootstrap config router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace prompt/bootstrap daemon facade broad seams", () => {
  const violations = scanWorkspacePromptBootstrapConfigDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/prompt_bootstrap_config.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
          let _ = self.state.global_store();
          let _ = daemon;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes(
      "workspace prompt/bootstrap config daemon facade uses broad daemon state",
    ),
  );
  assert(
    violations.includes(
      "workspace prompt/bootstrap config daemon facade uses broad daemon handle",
    ),
  );
  assert(
    violations.includes(
      "workspace prompt/bootstrap config daemon facade uses broad workspace handle",
    ),
  );
  assert(
    violations.includes(
      "workspace prompt/bootstrap config daemon facade exposes generic state/daemon escape hatch",
    ),
  );
});

test("appstate guard rejects workspace prompt/bootstrap broad handle fields", () => {
  const fieldViolations = scanWorkspacePromptBootstrapConfigHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspacePromptBootstrapConfigHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace prompt/bootstrap config capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace prompt/bootstrap config capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspacePromptBootstrapConfigHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspacePromptBootstrapConfigHandle {
        workspace_stores: ProtectedWorkspaceStoreLookup,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace execution config extraction outside allowed route", () => {
  const violations = scanWorkspaceExecutionConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceExecutionConfigHandle;
      async fn handler(
        State(state): State<WorkspaceExecutionConfigHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace execution config route extracts WorkspaceExecutionConfigHandle outside execution config route",
  ]);

  assert.deepEqual(
    scanWorkspaceExecutionConfigRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
      contents: `
        async fn get_execution_config(
          State(state): State<WorkspaceExecutionConfigHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace execution config broad route and router composition", () => {
  const routeViolations = scanWorkspaceExecutionConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      pub(in crate::api) async fn get_execution_config(
        State(workspaces): State<WorkspacesHandle>,
        Path(id): Path<String>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routeViolations, [
    "workspace execution config route uses broad workspace handle",
  ]);

  const routerViolations = scanWorkspaceExecutionConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_execution_config: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace execution config router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace execution config daemon facade broad seams", () => {
  const violations = scanWorkspaceExecutionConfigDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/execution_config.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
          let _ = self.state.global_store();
          let _ = settings::load_settings(state.as_ref()).await;
          let _ = self.shared_vm_container_runtime_available();
          let _ = daemon;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace execution config daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace execution config daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace execution config daemon facade uses broad workspace handle"),
  );
  assert(
    violations.includes(
      "workspace execution config daemon facade reuses full-state settings helper",
    ),
  );
  assert(
    violations.includes(
      "workspace execution config daemon facade reuses full-state sandbox availability helper",
    ),
  );
  assert(
    violations.includes(
      "workspace execution config daemon facade exposes generic state/daemon escape hatch",
    ),
  );
});

test("appstate guard rejects workspace execution config broad handle fields", () => {
  const fieldViolations = scanWorkspaceExecutionConfigHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceExecutionConfigHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace execution config capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace execution config capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceExecutionConfigHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceExecutionConfigHandle {
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        data_root: PathBuf,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace file completions extraction outside allowed route", () => {
  const violations = scanWorkspaceFileCompletionsRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceFileCompletionsHandle;
      async fn handler(
        State(state): State<WorkspaceFileCompletionsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace file completions route extracts WorkspaceFileCompletionsHandle outside file completions route",
  ]);

  assert.deepEqual(
    scanWorkspaceFileCompletionsRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/management/file_completions.rs",
      contents: `
        async fn workspace_file_completions(
          State(state): State<WorkspaceFileCompletionsHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace file completions broad route and router composition", () => {
  const routeViolations = scanWorkspaceFileCompletionsRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/file_completions.rs",
    contents: `
      pub(in crate::api) async fn workspace_file_completions(
        State(workspaces): State<WorkspacesHandle>,
        Path(id): Path<String>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routeViolations, [
    "workspace file completions route uses broad workspace handle",
  ]);

  const routerViolations = scanWorkspaceFileCompletionsRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_file_completions: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace file completions router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace file completions daemon facade broad seams", () => {
  const violations = scanWorkspaceFileCompletionsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/workspace_file_completions_route.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
          let _ = self.state.global_store();
          let _ = daemon;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace file completions daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace file completions daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace file completions daemon facade uses broad workspace handle"),
  );
  assert(
    violations.includes(
      "workspace file completions daemon facade exposes generic state/daemon escape hatch",
    ),
  );
});

test("appstate guard rejects workspace file completions broad handle fields", () => {
  const fieldViolations = scanWorkspaceFileCompletionsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceFileCompletionsHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace file completions capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace file completions capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceFileCompletionsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceFileCompletionsHandle {
        global_store: Store,
        workspace_file_completions_cache: WorkspaceFileCompletionsCache,
        perf_telemetry: PerfTelemetry,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace harness container extraction outside allowed route", () => {
  const violations = scanWorkspaceHarnessContainerRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceHarnessContainerHandle;
      async fn handler(
        State(state): State<WorkspaceHarnessContainerHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace harness container route extracts WorkspaceHarnessContainerHandle outside harness container route",
  ]);

  assert.deepEqual(
    scanWorkspaceHarnessContainerRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
      contents: `
        async fn get_workspace_harness_container(
          State(state): State<WorkspaceHarnessContainerHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace harness container broad route and router composition", () => {
  const routeViolations = scanWorkspaceHarnessContainerRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
    contents: `
      pub(in crate::api) async fn get_workspace_harness_container(
        State(workspaces): State<WorkspacesHandle>,
        Path(id): Path<String>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routeViolations, [
    "workspace harness container route uses broad workspace handle",
  ]);

  const routerViolations = scanWorkspaceHarnessContainerRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_harness_container: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace harness container router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace harness container daemon facade broad seams", () => {
  const violations = scanWorkspaceHarnessContainerDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/harness_container.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
          let _ = self.state.global_store();
          let _ = execution_effective::effective_execution_settings_classified(state.as_ref(), workspace_id).await;
          let _ = daemon;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace harness container daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace harness container daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace harness container daemon facade uses broad workspace handle"),
  );
  assert(
    violations.includes(
      "workspace harness container daemon facade reuses full-state execution settings helper",
    ),
  );
  assert(
    violations.includes(
      "workspace harness container daemon facade exposes generic state/daemon escape hatch",
    ),
  );
});

test("appstate guard rejects workspace harness container broad handle fields", () => {
  const fieldViolations = scanWorkspaceHarnessContainerHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceHarnessContainerHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace harness container capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace harness container capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceHarnessContainerHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceHarnessContainerHandle {
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        daemon_url: String,
        harness: Arc<HarnessRuntimeManager>,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace worktree extraction outside allowed route", () => {
  const violations = scanWorkspaceWorktreeRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceWorktreeHandle;
      async fn handler(
        State(state): State<WorkspaceWorktreeHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace worktree route extracts WorkspaceWorktreeHandle outside worktree route",
  ]);

  assert.deepEqual(
    scanWorkspaceWorktreeRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/worktrees.rs",
      contents: `
        async fn get_worktree(
          State(state): State<WorkspaceWorktreeHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace worktree broad route and router composition", () => {
  const routeViolations = scanWorkspaceWorktreeRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/worktrees.rs",
    contents: `
      pub(in crate::api) async fn get_worktree(
        State(workspaces): State<WorkspacesHandle>,
        Path(id): Path<String>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routeViolations, [
    "workspace worktree route uses broad workspace handle",
  ]);

  const routerViolations = scanWorkspaceWorktreeRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_worktree: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace worktree router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace worktree daemon facade broad seams", () => {
  const violations = scanWorkspaceWorktreeDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/worktrees.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
          let _ = self.state.global_store();
          let _ = daemon;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace worktree daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace worktree daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace worktree daemon facade uses broad workspace handle"),
  );
  assert(
    violations.includes(
      "workspace worktree daemon facade exposes generic state/daemon escape hatch",
    ),
  );
});

test("appstate guard rejects legacy workspace worktree facades on broad handle", () => {
  const violations = scanWorkspaceWorktreeDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn get_worktree_with_live_root(&self) {}
        pub async fn get_worktree_bootstrap_log_path(&self) {}
        pub async fn download_worktree_bootstrap_logs_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace worktree route facade remains on broad workspace handle",
    "workspace worktree route facade remains on broad workspace handle",
    "workspace worktree route facade remains on broad workspace handle",
  ]);
});

test("appstate guard rejects workspace worktree broad handle fields", () => {
  const fieldViolations = scanWorkspaceWorktreeHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceWorktreeHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace worktree capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace worktree capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceWorktreeHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceWorktreeHandle {
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        data_root: PathBuf,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace registry extraction outside allowed routes", () => {
  const violations = scanWorkspaceRegistryRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceRegistryHandle;
      async fn handler(
        State(state): State<WorkspaceRegistryHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace registry route extracts WorkspaceRegistryHandle outside registry read/create route",
  ]);

  assert.deepEqual(
    scanWorkspaceRegistryRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
      contents: `
        async fn list_workspaces(
          State(registry): State<WorkspaceRegistryHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace registry broad route and router composition", () => {
  const routeViolations = scanWorkspaceRegistryRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      pub(in crate::api) async fn list_workspaces(
        State(workspaces): State<WorkspacesHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routeViolations, [
    "workspace registry read/create route uses broad workspace handle",
  ]);

  const routerViolations = scanWorkspaceRegistryRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_registry: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace registry router composed from broad workspace handle",
  ]);

  const deleteViolations = scanWorkspaceRegistryRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      pub(in crate::api) async fn delete_workspace(
        State(registry): State<WorkspaceRegistryHandle>,
      ) {
        registry.delete_workspace_for_route(params).await?;
      }
    `,
  }).map((violation) => violation.name);

  assert(
    deleteViolations.includes("workspace registry read/create route owns delete behavior"),
  );
});

test("appstate guard rejects workspace registry daemon facade broad seams", () => {
  const violations = scanWorkspaceRegistryDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/registry.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
          let _ = self.state.global_store();
          let _ = daemon;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace registry daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace registry daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace registry daemon facade uses broad workspace handle"),
  );
  assert(
    violations.includes(
      "workspace registry daemon facade exposes generic state/daemon escape hatch",
    ),
  );

  const deleteViolations = scanWorkspaceRegistryDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/registry.rs",
    contents: `
      impl WorkspaceRegistryHandle {
        pub async fn delete_workspace_for_route(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(deleteViolations, [
    "workspace registry read/create daemon facade owns delete behavior",
  ]);
});

test("appstate guard rejects legacy workspace registry read/create facades on broad handle", () => {
  const violations = scanWorkspaceRegistryDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_config.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn list_workspaces_for_route(&self) {}
        pub async fn get_workspace_for_route_params(&self) {}
        pub async fn get_workspace_for_route(&self) {}
        pub async fn create_workspace_for_request(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace registry read/create facade remains on broad workspace handle",
    "workspace registry read/create facade remains on broad workspace handle",
    "workspace registry read/create facade remains on broad workspace handle",
    "workspace registry read/create facade remains on broad workspace handle",
  ]);
});

test("appstate guard rejects workspace registry broad handle fields", () => {
  const fieldViolations = scanWorkspaceRegistryHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceRegistryHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace registry capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace registry capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceRegistryHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceRegistryHandle {
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        telemetry: Telemetry,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace deletion extraction outside delete route", () => {
  const violations = scanWorkspaceDeletionRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      async fn handler(
        State(state): State<WorkspaceDeletionHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace deletion route extracts WorkspaceDeletionHandle outside registry delete route",
  ]);

  assert.deepEqual(
    scanWorkspaceDeletionRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/registry/delete.rs",
      contents: `
        async fn delete_workspace(
          State(deletion): State<WorkspaceDeletionHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects post-slice workspace deletion WorkspacesHandle HTTP seams", () => {
  const routeContents = `
    use ctx_daemon::daemon::WorkspacesHandle;
    pub(in crate::api) async fn delete_workspace(
      State(workspaces): State<WorkspacesHandle>,
    ) {}
  `;

  assert.deepEqual(
    scanWorkspaceDeletionRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/workspaces/registry/delete.rs",
      contents: routeContents,
      workspaceDeletionCapability: false,
    }),
    [],
  );

  const routeViolations = scanWorkspaceDeletionRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry/delete.rs",
    contents: routeContents,
    workspaceDeletionCapability: true,
  }).map((violation) => violation.name);

  assert(
    routeViolations.includes("workspace deletion route uses broad workspace handle"),
  );

  const regressedRouteViolations = scanWorkspaceDeletionRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry/delete.rs",
    contents: routeContents,
    workspaceDeletionCapability: true,
    workspaceDeletionRouteMigrated: false,
  }).map((violation) => violation.name);

  assert(
    regressedRouteViolations.includes(
      "workspace deletion route uses broad workspace handle",
    ),
  );

  const routerViolations = scanWorkspaceDeletionRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      pub(in crate::api) struct RouteHandles {
        workspace_deletion: WorkspacesHandle,
      }
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_deletion: handle.workspaces(),
          }
        }
      }
    `,
    workspaceDeletionCapability: true,
  }).map((violation) => violation.name);

  assert(
    routerViolations.includes(
      "ctx-http carries broad workspace handle after workspace deletion slice",
    ),
  );
  assert(
    routerViolations.includes(
      "workspace deletion router composed from broad workspace handle",
    ),
  );
});

test("appstate guard rejects workspace deletion route facade on WorkspacesHandle", () => {
  assert.deepEqual(
    scanWorkspaceDeletionDaemonImplementationRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/registry_delete.rs",
      contents: `
        impl WorkspacesHandle {
          pub async fn delete_workspace_for_route(&self) {}
        }
      `,
      workspaceDeletionCapability: false,
    }),
    [],
  );

  const violations = scanWorkspaceDeletionDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/registry_delete.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn delete_workspace_for_route(&self) {}
      }
    `,
    workspaceDeletionCapability: true,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace deletion route facade remains on WorkspacesHandle",
  ]);

  const backdoorViolations = scanWorkspaceDeletionDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/deletion.rs",
    contents: `
      pub async fn delete_workspace(
        state: &Arc<DaemonState>,
        workspace_id: WorkspaceId,
      ) {}
    `,
    workspaceDeletionCapability: true,
  }).map((violation) => violation.name);

  assert.deepEqual(backdoorViolations, [
    "workspace deletion exposes public DaemonState delete backdoor",
  ]);
});

test("appstate guard rejects workspace deletion broad handle and runtime fields", () => {
  const handleViolations = scanWorkspaceDeletionHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceDeletionHandle {
        state: Arc<DaemonState>,
        daemon: DaemonHandle,
        workspaces: WorkspacesHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    handleViolations.includes(
      "workspace deletion capability stores broad handle or daemon state",
    ),
  );
  assert(
    handleViolations.includes(
      "workspace deletion capability exposes generic full-state escape hatch",
    ),
  );

  const runtimeViolations = scanWorkspaceDeletionHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/deletion/runtime.rs",
    contents: `
      pub(crate) struct WorkspaceDeletionRuntime {
        daemon: DaemonHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    runtimeViolations.includes(
      "workspace deletion runtime stores broad handle or daemon state",
    ),
  );
  assert(
    runtimeViolations.includes(
      "workspace deletion runtime exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = [
    ...scanWorkspaceDeletionHandleFieldRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        pub struct WorkspaceDeletionHandle {
          global_store: Store,
          workspace_stores: ProtectedWorkspaceStoreLookup,
          runtime: Arc<WorkspaceDeletionRuntime>,
        }
      `,
    }),
    ...scanWorkspaceDeletionHandleFieldRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/deletion/runtime.rs",
      contents: `
        pub(crate) struct WorkspaceDeletionRuntime {
          data_root: PathBuf,
          global_store: Store,
          workspace_stores: ProtectedWorkspaceStoreLookup,
        }
      `,
    }),
  ];

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace merge queue config extraction outside allowed route", () => {
  const violations = scanWorkspaceMergeQueueConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      async fn handler(
        State(state): State<WorkspaceMergeQueueConfigHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace merge queue config route extracts WorkspaceMergeQueueConfigHandle outside merge queue config route",
  ]);
});

test("appstate guard rejects workspace merge queue config broad route and router composition", () => {
  const routeViolations = scanWorkspaceMergeQueueConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      pub(in crate::api) async fn get_merge_queue_config(
        State(workspaces): State<WorkspacesHandle>,
      ) {}
      pub(in crate::api) async fn update_merge_queue_config(
        State(mut merge_queue_config): State<WorkspaceMergeQueueConfigHandle>,
      ) {}
      pub(in crate::api) async fn get_execution_config(
        State(merge_queue_config): State<WorkspaceMergeQueueConfigHandle>,
      ) {}
      pub(in crate::api) async fn get_workspace_primary_branch(
        mut merge_queue_config: State<WorkspaceMergeQueueConfigHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert(
    routeViolations.includes("workspace merge queue config route uses broad workspace handle"),
  );
  assert.equal(
    routeViolations.filter(
      (name) => name === "workspace merge queue config handle used outside merge queue config route",
    ).length,
    2,
  );

  const routerViolations = scanWorkspaceMergeQueueConfigRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_merge_queue_config: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace merge queue config router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace merge queue config daemon facade broad seams", () => {
  const violations = scanWorkspaceMergeQueueConfigDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/merge_queue_config.rs",
    contents: `
      use crate::daemon::{DaemonHandle, DaemonState, WorkspacesHandle};
      impl WorkspaceMergeQueueConfigHandle {
        pub async fn bad(&self, daemon: DaemonHandle, state: Arc<DaemonState>, workspaces: WorkspacesHandle) {
          self.state.clone();
          crate::daemon::merge_queue::schedule_workspace_if_enabled_and_queued(&state, WorkspaceId::new()).await;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace merge queue config daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace merge queue config daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace merge queue config daemon facade uses broad workspace handle"),
  );
  assert(
    violations.includes(
      "workspace merge queue config daemon facade reuses full-state merge queue helper",
    ),
  );
  assert(
    violations.includes(
      "workspace merge queue config daemon facade exposes generic state/daemon escape hatch",
    ),
  );
});

test("appstate guard rejects legacy workspace merge queue config facades on broad handle", () => {
  const violations = scanWorkspaceMergeQueueConfigDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/management.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn workspace_merge_queue_config_for_route(&self) {}
        pub async fn update_workspace_merge_queue_config_for_route(&self) {}
        pub async fn schedule_workspace_merge_queue_if_enabled_and_queued(&self) {}
        pub async fn cancel_queued_entries_for_disabled_workspace(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.equal(
    violations.filter(
      (name) => name === "workspace merge queue config facade remains on broad workspace handle",
    ).length,
    4,
  );
});

test("appstate guard rejects workspace merge queue config broad handle fields", () => {
  const fieldViolations = scanWorkspaceMergeQueueConfigHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceMergeQueueConfigHandle {
        state: Arc<DaemonState>,
        workspaces: WorkspacesHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace merge queue config capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace merge queue config capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceMergeQueueConfigHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceMergeQueueConfigHandle {
        workspace_stores: ProtectedWorkspaceStoreLookup,
        merge_queue: Arc<MergeQueueRuntime>,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace attachments extraction outside attachment routes", () => {
  const violations = scanWorkspaceAttachmentsRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      async fn handler(
        State(state): State<WorkspaceAttachmentsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace attachments route extracts WorkspaceAttachmentsHandle outside attachment routes",
  ]);
});

test("appstate guard rejects workspace attachments broad route and router composition", () => {
  const routeViolations = scanWorkspaceAttachmentsRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/attachments.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      pub(in crate::api) async fn list_workspace_attachments(
        State(workspaces): State<WorkspacesHandle>,
      ) {}
      pub(in crate::api) async fn sync_workspace_attachments(
        State(attachments): State<WorkspaceAttachmentsHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert(
    routeViolations.includes("workspace attachments route uses broad workspace handle"),
  );

  const routerViolations = scanWorkspaceAttachmentsRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_attachments: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace attachments router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace attachments daemon facade broad seams", () => {
  const violations = scanWorkspaceAttachmentsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/attachments.rs",
    contents: `
      use crate::daemon::{DaemonHandle, DaemonState, WorkspacesHandle};
      impl WorkspaceAttachmentsHandle {
        pub async fn bad(&self, daemon: DaemonHandle, state: Arc<DaemonState>, workspaces: WorkspacesHandle) {}
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace attachments daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace attachments daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace attachments daemon facade uses broad workspace handle"),
  );
});

test("appstate guard rejects legacy workspace attachment facades on broad handle", () => {
  const violations = scanWorkspaceAttachmentsDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn list_workspace_attachments(&self) {}
        pub async fn upsert_workspace_attachment(&self) {}
        pub async fn delete_workspace_attachment(&self) {}
        pub async fn sync_workspace_attachments(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.equal(
    violations.filter(
      (name) => name === "workspace attachment facade remains on broad workspace handle",
    ).length,
    4,
  );
});

test("appstate guard rejects workspace attachments broad handle and runtime fields", () => {
  const fieldViolations = scanWorkspaceAttachmentsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceAttachmentsHandle {
        state: Arc<DaemonState>,
        workspaces: WorkspacesHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace attachments capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace attachments capability exposes generic full-state escape hatch",
    ),
  );

  const runtimeViolations = scanWorkspaceAttachmentsHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/attachments/runtime.rs",
    contents: `
      pub(crate) struct WorkspaceAttachmentsRuntime {
        daemon: DaemonHandle,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    runtimeViolations.includes(
      "workspace attachments runtime stores broad handle or daemon state",
    ),
  );
  assert(
    runtimeViolations.includes(
      "workspace attachments runtime exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = [
    ...scanWorkspaceAttachmentsHandleFieldRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        pub struct WorkspaceAttachmentsHandle {
          global_store: Store,
          workspace_stores: ProtectedWorkspaceStoreLookup,
          runtime: Arc<WorkspaceAttachmentsRuntime>,
        }
      `,
    }),
    ...scanWorkspaceAttachmentsHandleFieldRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/attachments/runtime.rs",
      contents: `
        pub(crate) struct WorkspaceAttachmentsRuntime {
          data_root: PathBuf,
          daemon_url: String,
          global_store: Store,
          workspace_stores: ProtectedWorkspaceStoreLookup,
          harness: Arc<HarnessRuntimeManager>,
          materialization: Arc<WorkspaceAttachmentMaterializationRuntime>,
        }
      `,
    }),
  ];

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace composition raw-state helper seams", () => {
  const stateViolations = [
    ...scanWorkspaceCompositionStateCutRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/deletion.rs",
      contents: `
        use crate::daemon::DaemonState;
        pub fn runtime_from_state(state: &Arc<DaemonState>) {
          let _ = state.core.data_root.clone();
        }
      `,
    }),
    ...scanWorkspaceCompositionStateCutRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/task_worktree_host.rs",
      contents: `
        pub(in crate::daemon) struct TaskWorktreeHost {
          daemon: DaemonHandle,
          state: Arc<DaemonState>,
        }
      `,
    }),
    ...scanWorkspaceCompositionStateCutRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces.rs",
      contents: `
        use crate::daemon::DaemonState;
        pub fn sneaky_helper(state: &Arc<DaemonState>) {
          let _ = state.core.data_root.clone();
        }
      `,
    }),
    ...scanWorkspaceCompositionStateCutRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces.rs",
      contents: `
        mod worktree_bootstrap;
        mod worktree_provision;
        pub use worktree_bootstrap::spawn_worktree_bootstrap;
        pub use worktree_provision::provision_worktree_for_execution;
        pub use file_completions::complete_files_for_workspace;
        pub use attachments::ensure_worktree_attachment_mounts_if_materialized;
      `,
    }),
  ].map((violation) => violation.name);

  assert(
    stateViolations.includes("workspace composition file depends on broad daemon state"),
  );
  assert(
    stateViolations.includes("workspace composition file uses raw state field fanout"),
  );
  assert(
    stateViolations.includes("workspace composition file reintroduces raw-state runtime helper"),
  );
  assert(
    stateViolations.includes("workspace composition exports deleted raw-state workspace helper"),
  );

  const narrowViolations = scanWorkspaceCompositionStateCutRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/task_worktree_host.rs",
    contents: `
      pub(in crate::daemon) struct TaskWorktreeHost {
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        attachments: Arc<WorkspaceAttachmentsRuntime>,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects deleted workspace composition helper paths", () => {
  const violations = scanWorkspaceCompositionDeletedPathRatchet({
    existingRelativePaths: [
      "core/crates/ctx-daemon/src/daemon/workspaces/worktree_provision.rs",
      "core/crates/ctx-daemon/src/daemon/workspaces/worktree_bootstrap/command.rs",
    ],
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace composition deleted raw-state helper path exists",
    "workspace composition deleted raw-state helper path exists",
  ]);
});

test("appstate guard rejects workspace primary branch extraction outside allowed route", () => {
  const violations = scanWorkspacePrimaryBranchRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry.rs",
    contents: `
      async fn handler(
        State(state): State<WorkspacePrimaryBranchHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace primary branch route extracts WorkspacePrimaryBranchHandle outside management primary-branch route",
  ]);
});

test("appstate guard rejects workspace primary branch broad route and router composition", () => {
  const routeViolations = scanWorkspacePrimaryBranchRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      pub(in crate::api) async fn get_workspace_primary_branch(
        State(workspaces): State<WorkspacesHandle>,
      ) {}
      pub(in crate::api) async fn update_workspace_primary_branch(
        State(primary_branch): State<WorkspacePrimaryBranchHandle>,
      ) {}
      pub(in crate::api) async fn get_merge_queue_config(
        State(primary_branch): State<WorkspacePrimaryBranchHandle>,
      ) {}
      pub(in crate::api) async fn get_execution_config(
        State(primary_branch): State<WorkspacePrimaryBranchHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert(
    routeViolations.includes("workspace primary branch route uses broad workspace handle"),
  );
  assert(
    routeViolations.filter(
      (name) => name === "workspace primary branch handle used outside primary-branch route",
    ).length === 2,
  );

  const routerViolations = scanWorkspacePrimaryBranchRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_primary_branch: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace primary branch router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace primary branch daemon facade broad seams", () => {
  const violations = scanWorkspacePrimaryBranchDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/primary_branch.rs",
    contents: `
      use crate::daemon::{DaemonHandle, WorkspacesHandle};
      use crate::daemon::DaemonState;

      impl WorkspacesHandle {
        async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
          let _ = self.state.global_store();
          let _ = daemon;
          let force_emit = true;
        }
      }
    `,
  }).map((violation) => violation.name);

  assert(
    violations.includes("workspace primary branch daemon facade uses broad daemon state"),
  );
  assert(
    violations.includes("workspace primary branch daemon facade uses broad daemon handle"),
  );
  assert(
    violations.includes("workspace primary branch daemon facade uses broad workspace handle"),
  );
  assert(
    violations.includes(
      "workspace primary branch daemon facade exposes generic state/daemon escape hatch",
    ),
  );
  assert(
    violations.includes("workspace primary branch daemon facade exposes refresh force flag"),
  );
});

test("appstate guard rejects legacy workspace primary branch facades on broad handle", () => {
  const violations = scanWorkspacePrimaryBranchDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/management.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn update_workspace_primary_branch(&self) {}
        pub async fn load_workspace_primary_branch_config(&self) {}
        pub async fn update_workspace_primary_branch_config(&self) {}
        pub async fn workspace_primary_branch_for_request(&self) {}
        pub async fn update_workspace_primary_branch_for_request(&self) {}
        pub async fn refresh_worktree_vcs_snapshot(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.equal(
    violations.filter(
      (name) => name === "workspace primary branch facade remains on broad workspace handle",
    ).length,
    6,
  );
});

test("appstate guard rejects workspace primary branch broad handle fields", () => {
  const fieldViolations = scanWorkspacePrimaryBranchHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspacePrimaryBranchHandle {
        workspaces: WorkspacesHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace primary branch capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace primary branch capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspacePrimaryBranchHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspacePrimaryBranchHandle {
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        refresh_vcs_snapshot: WorkspacePrimaryBranchRefreshEffect,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects workspace provider model preference extraction outside allowed route", () => {
  const violations = scanWorkspaceProviderModelPreferenceRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      use ctx_daemon::daemon::WorkspaceProviderModelPreferenceHandle;
      async fn handler(
        State(state): State<WorkspaceProviderModelPreferenceHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "workspace provider model preference route extracts WorkspaceProviderModelPreferenceHandle outside provider preference route",
  ]);

  assert.deepEqual(
    scanWorkspaceProviderModelPreferenceRouteExtractorRatchet({
      filePath:
        "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
      contents: `
        async fn handler(
          State(state): State<WorkspaceProviderModelPreferenceHandle>,
        ) {}
      `,
    }),
    [],
  );
});

test("appstate guard rejects workspace provider model preference broad route and router composition", () => {
  const routeViolations = scanWorkspaceProviderModelPreferenceRouteExtractorRatchet({
    filePath:
      "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
    contents: `
      use ctx_daemon::daemon::WorkspacesHandle;
      async fn handler(State(state): State<WorkspacesHandle>) {}
    `,
  }).map((violation) => violation.name);

  assert(
    routeViolations.includes(
      "workspace provider model preference route uses broad workspace handle",
    ),
  );

  const routerViolations = scanWorkspaceProviderModelPreferenceRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/router.rs",
    contents: `
      impl RouteHandles {
        fn from_daemon_handle(handle: DaemonHandle) -> Self {
          Self {
            workspace_provider_model_preferences: handle.workspaces(),
          }
        }
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(routerViolations, [
    "workspace provider model preference router composed from broad workspace handle",
  ]);
});

test("appstate guard rejects workspace provider model preference daemon facade broad seams", () => {
  for (const filePath of [
    "core/crates/ctx-daemon/src/daemon/workspaces/model_preferences.rs",
    "core/crates/ctx-daemon/src/daemon/workspaces/provider_model_preferences_route.rs",
  ]) {
    const violations = scanWorkspaceProviderModelPreferenceDaemonImplementationRatchet({
      filePath,
      contents: `
        use crate::daemon::{DaemonHandle, WorkspacesHandle};
        use crate::daemon::DaemonState;

        impl WorkspacesHandle {
          async fn broad(&self, state: Arc<DaemonState>, daemon: DaemonHandle) {
            let _ = self.state.global_store();
            let _ = providers::effective_preferred_model_id_for_workspace();
            let _ = effective_preferred_model_id_for_workspace();
            let _ = daemon;
          }
        }
      `,
    }).map((violation) => violation.name);

    assert(
      violations.includes(
        "workspace provider model preference daemon facade uses broad daemon state",
      ),
    );
    assert(
      violations.includes(
        "workspace provider model preference daemon facade uses broad daemon handle",
      ),
    );
    assert(
      violations.includes(
        "workspace provider model preference daemon facade uses broad workspace handle",
      ),
    );
    assert(
      violations.includes(
        "workspace provider model preference daemon facade reuses full-state effective preference helper",
      ),
    );
    assert(
      violations.includes(
        "workspace provider model preference daemon facade exposes generic state/daemon escape hatch",
      ),
    );
  }
});

test("appstate guard rejects workspace provider model preference broad handle fields", () => {
  const fieldViolations = scanWorkspaceProviderModelPreferenceHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceProviderModelPreferenceHandle {
        workspaces: WorkspacesHandle,
        provider_options: ProviderOptionsHandle,
        daemon: DaemonHandle,
        state: Arc<DaemonState>,
      }
    `,
  }).map((violation) => violation.name);

  assert(
    fieldViolations.includes(
      "workspace provider model preference capability stores broad handle or daemon state",
    ),
  );
  assert(
    fieldViolations.includes(
      "workspace provider model preference capability exposes generic full-state escape hatch",
    ),
  );

  const narrowViolations = scanWorkspaceProviderModelPreferenceHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceProviderModelPreferenceHandle {
        launch: Arc<ProviderWorkspaceLaunchRuntime>,
      }
    `,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects raw store access in workspace REST/config route helpers", () => {
  const violations = scanWorkspaceRestConfigStoreIntentRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/registry.rs",
    contents: `
      use ctx_store::Store;

      async fn route(handle: WorkspaceRegistryHandle, workspace_id: WorkspaceId) {
        let _typed: Option<ctx_store::Store> = None;
        let _ = handle.global_store().list_workspaces().await;
        let _ = handle.existing_workspace_store(workspace_id).await;
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("workspace REST/config route helper imports raw ctx_store Store"));
  assert(
    violations.includes("workspace REST/config route helper uses raw global store accessor"),
  );
  assert(
    violations.includes("workspace REST/config route helper uses raw workspace store accessor"),
  );

  const worktreeViolations = scanWorkspaceRestConfigStoreIntentRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/worktrees.rs",
    contents: `
      async fn route(handle: WorkspaceWorktreeHandle, worktree_id: WorktreeId) {
        let _ = handle.store_for_worktree(worktree_id).await;
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(worktreeViolations, [
    "workspace REST/config route helper uses raw worktree store accessor",
  ]);

  const settingsViolations = scanWorkspaceRestConfigStoreIntentRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/route_contract/harness_container.rs",
    contents: `
      async fn route(handle: WorkspaceHarnessContainerHandle, store: Store) {
        let _ = ctx_settings_service::effective_execution_settings_classified(
          handle.global_store(),
          &store,
        ).await;
      }
    `,
  }).map((violation) => violation.name);
  assert(
    settingsViolations.includes(
      "workspace REST/config route helper resolves execution settings directly",
    ),
  );
  const settingsPrefixViolations = scanWorkspaceRestConfigStoreIntentRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/execution_config.rs",
    contents: `
      async fn route(store: Store) {
        let _ = ctx_settings_service::effective_execution_settings_for_environment(
          &store,
        ).await;
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(settingsPrefixViolations, [
    "workspace REST/config route helper resolves execution settings directly",
  ]);

  const launchViolations = scanWorkspaceRestConfigStoreIntentRatchet({
    filePath:
      "core/crates/ctx-daemon/src/daemon/workspaces/provider_model_preferences_route.rs",
    contents: `
      async fn route(handle: WorkspaceProviderModelPreferenceHandle) {
        let _ = handle.launch().providers();
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(launchViolations, [
    "workspace provider model preference route reaches through launch runtime",
  ]);
});

test("appstate guard ignores workspace REST/config fixture store setup after cfg-test stripping", () => {
  const contents = stripCfgTestItems(`
    impl WorkspaceExecutionConfigHandle {
      pub async fn route(&self, params: WorkspaceRouteParams) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.require_workspace_execution_config_update_target(workspace_id).await
      }
    }

    #[cfg(test)]
    mod tests {
      async fn fixture(daemon: TestDaemon, workspace_id: WorkspaceId) {
        let _ = daemon.global_store().get_workspace(workspace_id).await;
        let _ = daemon.store_for_workspace(workspace_id).await;
      }
    }
  `);

  assert.deepEqual(
    scanWorkspaceRestConfigStoreIntentRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/execution_config.rs",
      contents,
    }),
    [],
  );
});

test("appstate guard rejects raw store access in terminal and web-session launch helpers", () => {
  const strictViolations = scanTerminalWebSessionLaunchStoreIntentRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch/worktree.rs",
    contents: `
      use ctx_store::Store;

      async fn resolve(handle: TerminalLaunchHost, worktree_id: WorktreeId) {
        let _typed: Option<ctx_store::Store> = None;
        let _ = handle.global_store().get_workspace(workspace_id).await;
        let _ = handle.global_store.get_workspace(workspace_id).await;
        let _ = handle.store_for_worktree(worktree_id).await;
        let _ = ctx_settings_service::effective_execution_settings_for_environment(
          &store,
        ).await;
      }
    `,
  }).map((violation) => violation.name);

  assert(
    strictViolations.includes(
      "terminal/web-session launch helper imports raw ctx_store Store",
    ),
  );
  assert(
    strictViolations.includes(
      "terminal/web-session launch helper uses raw global store accessor",
    ),
  );
  assert(
    strictViolations.includes(
      "terminal/web-session launch helper uses raw store lookup accessor",
    ),
  );
  assert(
    strictViolations.includes(
      "terminal/web-session launch helper resolves execution settings directly",
    ),
  );

  const hostViolations = scanTerminalWebSessionLaunchStoreIntentRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/web_sessions/launch.rs",
    contents: `
      impl WebSessionLaunchHost {
        async fn store_for_session(&self, session_id: SessionId) -> anyhow::Result<Store> {
          self.workspace_stores.store_for_session(session_id).await
        }
      }
    `,
  }).map((violation) => violation.name);
  assert.deepEqual(hostViolations, [
    "terminal/web-session launch host exposes raw store accessor helper",
  ]);

  assert.deepEqual(
    scanTerminalWebSessionLaunchStoreIntentRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch/data_plane.rs",
      contents: `
        use ctx_store::Store;
        async fn workspace_store(host: TerminalLaunchHost, workspace_id: WorkspaceId) {
          let _ = host.workspace_stores.store_for_workspace(workspace_id).await;
        }
      `,
    }),
    [],
  );
});

test("appstate guard ignores terminal and web-session launch test fixtures after cfg-test stripping", () => {
  const contents = stripCfgTestItems(`
    pub(super) async fn resolve_terminal_worktree(host: &TerminalLaunchHost) -> anyhow::Result<()> {
      host.load_explicit_terminal_worktree(workspace_id, worktree_id).await?;
      Ok(())
    }

    #[cfg(test)]
    mod tests {
      async fn fixture(host: TerminalLaunchHost, worktree_id: WorktreeId) {
        let _ = host.store_for_worktree(worktree_id).await;
      }
    }
  `);

  assert.deepEqual(
    scanTerminalWebSessionLaunchStoreIntentRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/terminals/launch/worktree.rs",
      contents,
    }),
    [],
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
        use ctx_daemon::daemon::workspaces::stream::{
          ReplayOutcome, WorkspaceStreamReplayStepHook,
          WorkspaceStreamReplayDrainHook, WorkspaceStreamSubscriptionResolutionError,
        };
        use ctx_daemon::daemon::workspaces::{
          stream::{ReplayOutcome, WorkspaceStreamSubscriptionResolutionError},
        };
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
      "workspace stream API imports moved replay contracts from daemon",
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

test("daemon boundary guard rejects moved workspace stream contract daemon imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
    contents: `
      use ctx_daemon::daemon::workspaces::stream::{
        WorkspaceStreamReplayStepHook, WorkspaceStreamReplayDrainHook,
        WorkspaceStreamSubscriptionResolutionError,
      };
      use ctx_daemon::daemon::workspaces::stream::*;
      use ctx_daemon::daemon::workspaces::{stream::*};
      use ctx_daemon::daemon::{workspaces::stream::*};
      use ctx_daemon::{daemon::workspaces::stream::*};
      fn handler(error: WorkspaceStreamSubscriptionResolutionError) {
        let _ = error;
      }
    `,
    patterns: WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS,
  });

  assert.equal(
    violations.filter(
      (violation) =>
        violation.name === "workspace stream API imports moved replay contracts from daemon",
    ).length >= 5,
    true,
  );
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

test("daemon boundary guard ratchets workspace VCS stream HTTP handles", () => {
  const broadViolations = scanWorkspaceVcsStreamRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/client.rs",
    contents: `
      async fn helper(state: WorkspacesHandle) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(broadViolations, ["workspace VCS route uses broad workspace handle"]);

  const revertedRouteViolations = scanWorkspaceVcsStreamRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs.rs",
    contents: `
      async fn workspace_vcs_ws(
        State(state): State<WorkspacesHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(revertedRouteViolations, ["workspace VCS route uses broad workspace handle"]);

  assert.deepEqual(
    scanWorkspaceVcsStreamRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/client.rs",
      contents: `
        async fn helper(state: WorkspacesHandle) {}
      `,
      workspaceVcsStreamCapability: false,
    }),
    [],
  );

  const extractorViolations = scanWorkspaceVcsStreamRouteExtractorRatchet({
    filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs/socket.rs",
    contents: `
      async fn handler(
        State(state): State<WorkspaceVcsStreamHandle>,
      ) {}
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(extractorViolations, [
    "workspace VCS route extracts WorkspaceVcsStreamHandle outside route module",
  ]);

  assert.deepEqual(
    scanWorkspaceVcsStreamRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs.rs",
      contents: `
        async fn workspace_vcs_ws(
          State(state): State<WorkspaceVcsStreamHandle>,
        ) {}
      `,
    }),
    [],
  );

  assert.deepEqual(
    scanWorkspaceVcsStreamRouteExtractorRatchet({
      filePath: "core/crates/ctx-http/src/api/ws/workspace_vcs/socket.rs",
      contents: `
        async fn handler(runtime: WorkspaceVcsRuntime) {
          let _ = runtime.summary_worktree_ids.len();
        }
      `,
    }),
    [],
  );
});

test("daemon boundary guard rejects broad workspace VCS stream handle/runtime fields", () => {
  const violations = scanWorkspaceVcsStreamHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct WorkspaceVcsStreamHandle {
        state: Arc<DaemonState>,
        daemon: DaemonHandle,
        workspaces: WorkspacesHandle,
      }

      pub(in crate::daemon) struct WorkspaceVcsStreamHandleParts {
        workspace_runtime: WorkspaceRuntime,
      }

      pub struct WorkspaceVcsStreamRuntime {
        state: DaemonState,
        workspace_runtime: WorkspaceRuntime,
      }
    `,
  }).map((violation) => violation.name);

  assert(violations.includes("workspace VCS stream capability stores broad handle or daemon state"));
  assert(violations.includes("workspace VCS stream runtime stores broad handle or daemon state"));

  assert.deepEqual(
    scanWorkspaceVcsStreamHandleFieldRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        pub struct WorkspaceVcsStreamHandle {
          store: Store,
          runtime: Arc<WorkspaceVcsStreamRuntime>,
        }

        pub struct WorkspaceVcsStreamRuntime {
          worktree_vcs_enabled: bool,
          snapshots: Arc<WorkspaceVcsSnapshotCache>,
        }
      `,
    }),
    [],
  );

  assert.deepEqual(
    scanWorkspaceVcsStreamHandleFieldRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
      contents: `
        pub struct WorkspaceVcsRuntime {
          workspace_runtime: WorkspaceRuntime,
          state: Arc<DaemonState>,
        }
      `,
    }),
    [],
  );
});

test("daemon boundary guard keeps workspace VCS stream route methods in stream module", () => {
  const outsideViolations = scanWorkspaceVcsStreamDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/management.rs",
    contents: `
      impl WorkspaceVcsStreamHandle {
        pub(in crate::daemon) fn new() -> Self {}

        pub async fn release_workspace_vcs_demand(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(outsideViolations, [
    "workspace VCS stream route method implemented outside VCS stream module",
  ]);

  const broadViolations = scanWorkspaceVcsStreamDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/workspaces/stream/vcs.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn require_workspace_vcs_stream_access(&self) {}
      }
    `,
  }).map((violation) => violation.name);

  assert.deepEqual(broadViolations, [
    "workspace VCS stream route method remains on broad workspace handle",
  ]);

  assert.deepEqual(
    scanWorkspaceVcsStreamDaemonImplementationRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/stream/vcs.rs",
      contents: `
        impl WorkspaceVcsStreamHandle {
          pub async fn require_workspace_vcs_stream_access(&self) {}
          pub fn plan_workspace_vcs_lag_reseed(&self) {}
        }
      `,
    }),
    [],
  );

  assert.deepEqual(
    scanWorkspaceVcsStreamDaemonImplementationRatchet({
      filePath: "core/crates/ctx-daemon/src/daemon/workspaces/stream/vcs.rs",
      contents: `
        impl WorkspacesHandle {
          pub async fn require_workspace_vcs_stream_access(&self) {}
        }
      `,
      workspaceVcsStreamCapability: false,
    }),
    [],
  );
});

test("daemon boundary guard rejects raw terminal stream runtime access in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/ws/terminal/socket.rs",
    contents: `
      use ctx_daemon::daemon::terminals::{
        TerminalStreamInitialSnapshot,
        TerminalStreamOutputRecv,
        TerminalStreamSession,
      };
      use ctx_daemon::daemon::terminals as daemon_terminals;
      use ctx_daemon::daemon::terminals::*;
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
      "terminal WS API imports terminal stream runtime from daemon",
      "terminal WS API imports daemon terminals root",
      "terminal WS API imports daemon terminals root",
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
      use ctx_daemon::daemon::{CoreHandle, DictationConfigError};
      use ctx_daemon::daemon::dictation::DictationConfigError;
      use ctx_daemon::{daemon::dictation::DictationConfigError};
      use ctx_daemon::{daemon::{dictation::DictationConfigError, CoreHandle}};
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
      "dictation WS API imports moved config error from daemon",
      "dictation WS API imports moved config error from daemon",
      "dictation WS API imports moved config error from daemon",
      "dictation WS API imports moved config error from daemon",
      "dictation WS API imports moved config error from daemon",
      "dictation WS API imports moved config error from daemon",
      "dictation WS API imports moved config error from daemon",
      "dictation WS API imports moved config error from daemon",
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
        let parsed = WorkspaceId(uuid::Uuid::parse_str(raw)?);
        let exists = state.workspace_exists(workspace_id).await?;
        let exists = WorkspaceStreamHandle::workspace_exists(state, workspace_id).await?;
        let exists = WorkspacesHandle::workspace_exists(workspaces, workspace_id).await?;
        state.require_workspace_active_stream_access(workspace_id).await?;
        Ok(())
      }

      fn mobile_secure_stream_access_status() {}
      fn terminal_stream_access_status() {}
      fn terminal_stream_tail_bytes() {}
      fn secure_query(query: MobileSecureStreamQuery) {
        let device_id = query.device_id.trim();
        let token = query.token.trim();
      }
    `,
    patterns: WORKSPACE_WS_ADMISSION_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "workspace WS API parses stream route ids directly",
      "workspace WS API performs direct workspace existence admission",
      "workspace WS API performs direct workspace existence admission",
      "workspace WS API performs direct workspace existence admission",
      "workspace WS API calls raw stream admission helpers",
      "workspace WS API defines local stream admission helper",
      "workspace WS API defines local stream admission helper",
      "workspace WS API defines local stream admission helper",
      "workspace WS API defines local stream admission helper",
      "workspace WS API trims secure mobile query fields directly",
      "workspace WS API trims secure mobile query fields directly",
    ],
  );
});

test("daemon boundary guard scopes workspace websocket admission ban", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/ws/secure_mobile.rs",
    "core/crates/ctx-http/src/api/ws/terminal.rs",
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
      "org policy API uses raw policy model contracts directly",
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
      "org policy API uses raw policy model contracts directly",
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
      use ctx_core::ids::OrgId;
      use ctx_core::models::{DaemonEnrollment, OrgPolicySnapshot, WorkspacePolicyOverlay};
      use ctx_daemon::daemon::org_policy::UpsertDaemonEnrollmentError;
      impl From<DaemonEnrollment> for DaemonEnrollmentResponse {
        fn from(enrollment: DaemonEnrollment) -> Self {
          Self {
            policy_signing_key_present: !enrollment.policy_signing_key.trim().is_empty(),
          }
        }
      }
      async fn handler(state: CoreHandle, enrollment: DaemonEnrollment) {
        let org_id = uuid::Uuid::parse_str(raw).map(OrgId)?;
        state.upsert_daemon_enrollment_checked(enrollment).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/enrollments.rs"),
  });
  assert.deepEqual(
    enrollmentViolations.map((violation) => violation.name),
    [
      "org policy API parses route ids directly",
      "org policy API parses route ids directly",
      "org policy API uses raw policy model contracts directly",
      "org policy API uses raw policy model contracts directly",
      "org policy API uses raw policy model contracts directly",
      "org policy API uses raw policy model contracts directly",
      "org policy API defines local route DTOs",
      "org policy API uses low-level policy errors directly",
      "org policy API calls low-level policy facades directly",
    ],
  );

  const routeViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/org_policy/enrollments.rs",
    contents: `
      use ctx_daemon::daemon::{
        CoreHandle,
        DaemonEnrollmentRouteResponse,
        DaemonEnrollmentsRouteResponse,
        OrgPolicyOrgRouteParams,
        OrgPolicyRouteError,
        OrgPolicyRouteErrorKind,
        UpsertDaemonEnrollmentRouteRequest,
      };
      async fn handler(state: CoreHandle, error: OrgPolicyRouteError) {
        let response: DaemonEnrollmentRouteResponse = state
          .upsert_daemon_enrollment_for_route(
            OrgPolicyOrgRouteParams::new(org_id),
            UpsertDaemonEnrollmentRouteRequest {},
          )
          .await?;
        let status = match error.kind() {
          OrgPolicyRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
          OrgPolicyRouteErrorKind::Conflict => StatusCode::CONFLICT,
          OrgPolicyRouteErrorKind::NotFound => StatusCode::NOT_FOUND,
          OrgPolicyRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/org_policy/enrollments.rs"),
  });
  assert.deepEqual(
    routeViolations.map((violation) => violation.name),
    ["org policy API imports route contracts from daemon"],
  );

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
      use ctx_repo_onboarding_service as service;
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
      "repo onboarding API calls repo onboarding service directly",
      "repo onboarding API calls repo onboarding service directly",
      "repo onboarding API calls repo onboarding service directly",
      "repo onboarding API calls repo onboarding service directly",
      "repo onboarding API calls repo onboarding service directly",
      "repo onboarding API calls repo onboarding service directly",
      "repo onboarding API calls repo onboarding service directly",
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
      #[derive(Deserialize)]
      struct RepoInitReq {
        path: String,
      }
      #[derive(Serialize)]
      struct RepoInitResp {
        path: String,
      }
      async fn handler(workspaces: WorkspacesHandle, error: RepoOnboardingError) {
        let path = workspaces.initialize_repo(DaemonRepoInitRequest {
          path: req.path,
          allow_existing: false,
          allow_non_empty: false,
        }).await?;
        let path = WorkspacesHandle::clone_repo(&workspaces, req).await?;
        let path = path.to_string_lossy().to_string();
        let status = match error.kind() {
          RepoOnboardingErrorKind::BadRequest => StatusCode::BAD_REQUEST,
          RepoOnboardingErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ApiErrorResp { error: error.message().to_string() };
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/repo/init.rs"),
  });
  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "repo onboarding API uses low-level daemon onboarding DTOs directly",
      "repo onboarding API uses low-level daemon onboarding DTOs directly",
      "repo onboarding API defines local route DTOs",
      "repo onboarding API defines local route DTOs",
      "repo onboarding API inspects low-level daemon onboarding errors directly",
      "repo onboarding API inspects low-level daemon onboarding errors directly",
      "repo onboarding API inspects low-level daemon onboarding errors directly",
      "repo onboarding API calls low-level daemon onboarding facade directly",
      "repo onboarding API calls low-level daemon onboarding facade directly",
      "repo onboarding API stringifies paths directly",
    ],
  );

  const routeViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/repo/init.rs",
    contents: `
      use ctx_route_contracts::repo_onboarding::{
        RepoInitRouteRequest,
        RepoOnboardingRouteError,
        RepoOnboardingRouteErrorKind,
        RepoPathRouteResponse,
      };
      async fn handler(workspaces: WorkspacesHandle, error: RepoOnboardingRouteError) {
        let response: RepoPathRouteResponse = workspaces
          .initialize_repo_for_route(RepoInitRouteRequest {})
          .await?;
        let status = match error.kind() {
          RepoOnboardingRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
          RepoOnboardingRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ApiErrorResp { error: error.message().to_string() };
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/repo/init.rs"),
  });
  assert.deepEqual(routeViolations, []);

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
        api::router(api::RouteHandles::from_daemon_route_handles(
          daemon.route_handles(),
        ))
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
        api::router(api::RouteHandles::from_daemon_route_handles(daemon.route_handles()))
      }
      pub fn other_router(daemon: &TestDaemon) -> axum::Router {
        api::router(api::RouteHandles::from_daemon_route_handles(daemon.route_handles()))
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
      filePath: "core/crates/ctx-http/src/test_support.rs",
      allowed: `
        pub(crate) fn router(&self) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_route_handles(
            self.daemon.route_handles(),
          ))
        }
      `,
      denied: `
        pub(crate) fn other_router(&self) -> axum::Router {
          crate::api::router(crate::api::RouteHandles::from_daemon_route_handles(self.daemon.route_handles()))
        }
      `,
    },
    {
      filePath: "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs",
      allowed: `
        pub(crate) fn spawn_router_for_daemon(listener: tokio::net::TcpListener, daemon: &TestDaemon) {
          let app = ctx_http::api::router(ctx_http::api::RouteHandles::from_daemon_route_handles(daemon.route_handles()));
        }
      `,
      denied: `
        pub(crate) fn other_router(daemon: &TestDaemon) {
          let app = ctx_http::api::router(ctx_http::api::RouteHandles::from_daemon_route_handles(daemon.route_handles()));
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

test("daemon boundary guard rejects mobile access API orchestration leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/mobile_access/secure.rs",
    contents: `
      use ctx_transport_runtime::mobile_e2ee;
      async fn helper(state: CoreHandle) {
        let client = reqwest::Client::new();
        let url = std::env::var("CTX_TUNNEL_CONTROL_PLANE_URL")?;
        let _token = generate_mobile_api_token();
        let _pairing_hash = hash_pairing_token("secret");
        let _key = mobile_e2ee::derive_key("device", "pub", "priv")?;
        let _cfg = state.get_mobile_access_config().await?;
        state.upsert_mobile_access_config(config).await?;
        state.create_mobile_connection_profile(label, base, hash, prefix, scopes).await?;
        state.update_mobile_connection_profile_scopes(profile_id, scopes).await?;
        state.insert_mobile_pairing_token(id, hash, expires_at).await?;
        state.consume_mobile_pairing_token(hash).await?;
        state.get_mobile_device(device_id).await?;
        state.upsert_mobile_device(device_id, profile_id, update).await?;
        state.advance_mobile_device_seq(device_id, seq).await?;
        state.load_mobile_auth_context_for_profile(profile_id).await?;
        let _scopes = mobile_scope_set_from_strings(&raw)?;
        let update = MobileDeviceRegistrationUpdate::default();
        let _router = SecureProxyRouterState;
        dispatch_scoped_secure_proxy_request(&state, method, uri, headers, mobile_auth).await;
        mobile_secure_proxy_allows_request(&method, &path);
        secure_proxy_path_is_unnormalized(&path);
        mobile_auth.allows(MobileScope::WorkspaceRead);
        desktop_auth_required_secure_response()?;
        mobile_scope_required_secure_response(MobileScope::WorkspaceRead)?;
        if path == "/api/health" {}
        if path == "/api/workspaces" {}
        let _workspace = path.strip_prefix("/api/workspaces/");
        health(State(core), headers).await?;
        list_workspaces(State(workspaces)).await?;
        get_workspace(State(workspaces), Path(id)).await?;
        let mut headers = HeaderMap::new();
        HeaderName::from_bytes(name.as_bytes())?;
        HeaderValue::from_str(value)?;
        let _bytes = to_bytes(resp.into_body(), 16).await?;
      }
    `,
    patterns: MOBILE_ACCESS_ORCHESTRATION_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "mobile access API calls control plane directly",
      "mobile access API calls control plane directly",
      "mobile access API owns mobile token helpers",
      "mobile access API owns mobile token helpers",
      "mobile access API owns mobile E2EE orchestration",
      "mobile access API owns mobile E2EE orchestration",
      "mobile access API calls raw mobile access config facade",
      "mobile access API calls raw mobile access config facade",
      "mobile access API calls raw mobile profile facade",
      "mobile access API calls raw mobile profile facade",
      "mobile access API calls raw mobile pairing facade",
      "mobile access API calls raw mobile pairing facade",
      "mobile access API calls raw mobile device facade",
      "mobile access API calls raw mobile device facade",
      "mobile access API calls raw mobile device facade",
      "mobile access API calls raw mobile auth context facade",
      "mobile access API owns mobile scope parsing or defaults",
      "mobile access API references raw mobile route DTOs",
      "mobile access API owns secure proxy router state",
      "mobile access API owns secure proxy router state",
      "mobile access API owns secure proxy transport admission",
      "mobile access API owns secure proxy transport admission",
      "mobile access API owns mobile secure proxy scope checks",
      "mobile access API owns mobile secure proxy scope checks",
      "mobile access API builds secure proxy denial responses",
      "mobile access API builds secure proxy denial responses",
      "mobile access API owns secure proxy path dispatch",
      "mobile access API owns secure proxy path dispatch",
      "mobile access API owns secure proxy path dispatch",
      "mobile access API re-enters proxied route handlers",
      "mobile access API re-enters proxied route handlers",
      "mobile access API re-enters proxied route handlers",
      "mobile access API owns secure proxy response reassembly",
      "mobile access API owns secure proxy request header filtering",
      "mobile access API owns secure proxy request header filtering",
      "mobile access API owns secure proxy request header filtering",
    ],
  );
});

test("daemon boundary guard rejects moved mobile access contracts from daemon", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/mobile_access.rs",
    contents: `
      use ctx_daemon::daemon::{
        mobile_access::{
          EnableMobileAccessRequest,
          MobileAccessRouteError,
          MobileSecureStreamContext,
        },
        CoreHandle,
      };
      async fn handler() {
        let _ = ctx_daemon::daemon::mobile_access::PairMobileDeviceRequest;
      }
    `,
    patterns: [
      ...MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
      ...MOBILE_ACCESS_ORCHESTRATION_API_PATTERNS,
      ...MOBILE_ACCESS_DAEMON_IMPORT_PATTERNS,
    ],
  });

  const names = violations.map((violation) => violation.name);
  assert(names.includes("mobile access API imports moved mobile contracts from daemon mobile_access"));
  assert(names.includes("mobile access API imports moved mobile contracts from nested daemon group"));
});

test("daemon boundary guard rejects moved mobile auth and stream contracts outside mobile_access routes", () => {
  for (const [filePath, contents] of [
    [
      "core/crates/ctx-http/src/api/mod.rs",
      `use ctx_daemon::daemon::{mobile_access::MobileAuthContext, CoreHandle};`,
    ],
    [
      "core/crates/ctx-http/src/api/auth.rs",
      `use ctx_daemon::daemon::{mobile_access::MobileAuthContext, CoreHandle};`,
    ],
    [
      "core/crates/ctx-http/src/api/auth/mobile.rs",
      `use ctx_daemon::daemon::{mobile_access::MobileAuthContext, CoreHandle};`,
    ],
    [
      "core/crates/ctx-http/src/api/ws/secure_mobile/context.rs",
      `use ctx_daemon::daemon::mobile_access::MobileSecureStreamContext;`,
    ],
    [
      "core/crates/ctx-http/src/api/ws/secure_mobile/socket.rs",
      `use ctx_daemon::daemon::{mobile_access::MobileSecureStreamContext, WorkspaceStreamHandle};`,
    ],
    [
      "core/crates/ctx-http/src/api/ws/secure_mobile.rs",
      `
        use ctx_daemon::daemon::{
          mobile_access::{MobileAccessRouteError, MobileSecureWorkspaceStreamRouteParams},
          CoreHandle,
        };
      `,
    ],
  ]) {
    const violations = scanText({
      filePath,
      contents,
      patterns: mobileAccessStoreDtoApiPatternsForPath(filePath),
    });
    assert(
      violations.some((violation) =>
        violation.name.startsWith("mobile access API imports moved mobile contracts"),
      ),
      `expected moved mobile contract import violation for ${filePath}`,
    );
  }
});

test("daemon boundary guard rejects daemon mobile_access root imports", () => {
  for (const [filePath, contents] of [
    [
      "core/crates/ctx-http/src/api/mod.rs",
      `use ctx_daemon::daemon::{mobile_access, CoreHandle};`,
    ],
    [
      "core/crates/ctx-http/src/api/auth.rs",
      `use ctx_daemon::daemon::mobile_access as daemon_mobile_access;`,
    ],
    [
      "core/crates/ctx-http/src/api/auth/mobile.rs",
      `use ctx_daemon::daemon::mobile_access::*;`,
    ],
    [
      "core/crates/ctx-http/src/api/ws/secure_mobile/context.rs",
      `use ctx_daemon::daemon::mobile_access::{self, MobileAuthContext};`,
    ],
    [
      "core/crates/ctx-http/src/api/ws/secure_mobile.rs",
      `
        use ctx_daemon::daemon::{
          mobile_access::*,
          CoreHandle,
        };
      `,
    ],
  ]) {
    const violations = scanText({
      filePath,
      contents,
      patterns: mobileAccessStoreDtoApiPatternsForPath(filePath),
    });
    assert(
      violations.some(
        (violation) => violation.name === "mobile access API imports daemon mobile_access root",
      ),
      `expected daemon mobile_access root import violation for ${filePath}`,
    );
  }
});

test("daemon boundary guard scopes mobile access storage DTO roots", () => {
  const mobileAccessPatterns = [
    ...MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
    ...MOBILE_ACCESS_ORCHESTRATION_API_PATTERNS,
    ...MOBILE_ACCESS_DAEMON_IMPORT_PATTERNS,
  ];
  for (const filePath of [
    "core/crates/ctx-http/src/api/mod.rs",
    "core/crates/ctx-http/src/api/mobile_access.rs",
    "core/crates/ctx-http/src/api/mobile_access/secure.rs",
    "core/crates/ctx-http/src/api/mobile_access/access_enable/profile_config.rs",
  ]) {
    assert.deepEqual(
      mobileAccessStoreDtoApiPatternsForPath(filePath),
      mobileAccessPatterns,
    );
  }
  assert.deepEqual(
    mobileAccessStoreDtoApiPatternsForPath("core/crates/ctx-http/src/api/providers/status.rs"),
    [],
  );
  assert.deepEqual(
    mobileAccessStoreDtoApiPatternsForPath("core/crates/ctx-http/src/api/auth.rs"),
    MOBILE_ACCESS_DAEMON_IMPORT_PATTERNS,
  );
  assert.deepEqual(
    mobileAccessStoreDtoApiPatternsForPath(
      "core/crates/ctx-http/src/api/ws/secure_mobile/context.rs",
    ),
    MOBILE_ACCESS_DAEMON_IMPORT_PATTERNS,
  );
});

test("daemon boundary guard rejects mobile profile route path-id parsing", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/mobile_access/profiles/connection_profiles.rs",
    contents: `
      async fn delete_profile(id: String) {
        let raw = uuid::Uuid::parse_str(&id)?;
        let _profile_id = ConnectionProfileId(raw);
        let other = Uuid::parse_str(&id)?;
      }
    `,
    patterns: MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "mobile profile route constructs ConnectionProfileId locally",
      "mobile profile route parses profile UUID locally",
      "mobile profile route parses profile UUID locally",
    ],
  );
});

test("daemon boundary guard scopes mobile profile path-id bans to profile routes", () => {
  assert.deepEqual(
    mobileProfileRouteApiPatternsForPath(
      "core/crates/ctx-http/src/api/mobile_access/profiles/connection_profiles.rs",
    ),
    MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS,
  );
  assert.deepEqual(
    mobileProfileRouteApiPatternsForPath(
      "core/crates/ctx-http/src/api/mobile_access/profiles/devices.rs",
    ),
    MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS,
  );
  assert.deepEqual(
    mobileProfileRouteApiPatternsForPath(
      "core/crates/ctx-http/src/api/mobile_access/secure.rs",
    ),
    [],
  );

  const routePatternNames = apiPatternsForPath(
    "core/crates/ctx-http/src/api/mobile_access/profiles/devices.rs",
  ).map((pattern) => pattern.name);
  assert.ok(routePatternNames.includes("mobile profile route parses profile UUID locally"));
  assert.ok(
    routePatternNames.includes("mobile profile route constructs ConnectionProfileId locally"),
  );
});

test("daemon boundary guard rejects route file download API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/artifacts/session/set.rs",
    contents: `
      async fn helper(state: SessionsHandle, path: PathBuf, root: PathBuf) {
        let _bytes = tokio::fs::read(&path).await?;
        let _meta = tokio::fs::metadata(&path).await?;
        let _canonical = tokio::fs::canonicalize(&path).await?;
        let _file = tokio::fs::File::open(&path).await?;
        let _open = std::fs::OpenOptions::new();
        let _safe = path_resolves_within_root(&path, &root).await;
        let _log_root = root.join("merge-queue");
        let _bootstrap_root = state.worktree_bootstrap_logs_root();
        let _worktree = state.get_session_worktree(&session).await?;
        let _spool = state.session_tool_output_spool_dir(session.id);
        let _path = resolve_session_artifact_accessible_path(&state, &session, &path).await?;
        let _write = validate_session_artifact_write_path(&state, &session, &path).await?;
        let _file = open_canonical_session_artifact_file(&path).await?;
        let _name = normalize_session_artifact_name(None, &path);
        let _etag = build_session_artifact_etag(42, modified);
      }
    `,
    patterns: ROUTE_FILE_DOWNLOAD_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "route file API reads or canonicalizes files directly",
      "route file API reads or canonicalizes files directly",
      "route file API reads or canonicalizes files directly",
      "route file API reads or canonicalizes files directly",
      "route file API owns symlink-safe open policy",
      "route file API calls local path root guard",
      "route file API reconstructs merge queue log root",
      "route file API calls worktree bootstrap log root facade",
      "route file API calls session artifact path facades directly",
      "route file API calls session artifact path facades directly",
      "route file API owns session artifact path authorization helpers",
      "route file API owns session artifact path authorization helpers",
      "route file API owns session artifact path authorization helpers",
      "route file API owns session artifact metadata derivation",
      "route file API owns session artifact metadata derivation",
    ],
  );
});

test("daemon boundary guard scopes route file download roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/artifacts/session/set.rs",
    "core/crates/ctx-http/src/api/artifacts/download/response.rs",
    "core/crates/ctx-http/src/api/merge_queue_api/logs.rs",
    "core/crates/ctx-http/src/api/workspaces/worktrees.rs",
  ]) {
    assert.deepEqual(
      routeFileDownloadApiPatternsForPath(filePath),
      ROUTE_FILE_DOWNLOAD_API_PATTERNS,
    );
  }
  assert.deepEqual(
    routeFileDownloadApiPatternsForPath("core/crates/ctx-http/src/api/mobile_access.rs"),
    [],
  );
});

test("daemon boundary guard rejects moved route DTO daemon imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/health.rs",
    contents: `
      use ctx_daemon::daemon::{
        CoreHandle, DaemonHealthSnapshot, HealthCompatibility, TextRouteDownload,
        WorkspaceHarnessContainerStatusRouteResponse, TelemetryExportError,
        TelemetryExportErrorKind,
      };
      use ctx_daemon::daemon::sessions::{
        DemoSeedTranscriptRouteRequest, DemoSeedTranscriptRouteResponse,
      };
      use ctx_daemon::{daemon::DaemonDiagnosticsSnapshot};
      use ctx_daemon::{daemon::{sessions::DemoSeedTranscriptRouteError, SessionsHandle}};
      use ctx_daemon::{daemon::{sessions::{DemoSeedTranscriptRouteErrorKind, DemoSeedTranscriptRouteTurn}}};
    `,
    patterns: ROUTE_DTO_SWEEP_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "route DTO sweep API imports moved route DTOs from daemon",
      "route DTO sweep API imports moved route DTOs from daemon",
      "route DTO sweep API imports moved route DTOs from daemon",
      "route DTO sweep API imports moved route DTOs from daemon",
      "route DTO sweep API imports moved route DTOs from daemon",
      "route DTO sweep API imports moved route DTOs from daemon",
      "route DTO sweep API imports moved route DTOs from daemon",
      "route DTO sweep API imports moved route DTOs from daemon",
    ],
  );
});

test("daemon boundary guard rejects stale daemon root route contract facades", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/mod.rs",
    contents: `
      use ctx_daemon::daemon::{
        RepoCloneRouteRequest, RepoStatusRouteResponse, SessionRouteParams,
        SessionHeadRouteResponse, WorkspaceRouteParams, WorkspaceRouteError,
        WorkspaceRouteResponse, WorktreeRouteParams,
      };
      use ctx_daemon::{daemon::{RepoOnboardingRouteError, SessionControlRouteError}};
      use ctx_daemon::daemon::sessions::SessionEventsRouteQuery;
      use ctx_daemon::daemon::workspaces::WorkspaceActiveSnapshotRouteResponse;
    `,
    patterns: ROUTE_DTO_SWEEP_API_PATTERNS,
  });

  assert(
    violations.length >= 4,
    `expected daemon root facade violations, saw ${JSON.stringify(violations)}`,
  );
  assert(
    violations.every(
      (violation) => violation.name === "route DTO sweep API imports moved route DTOs from daemon",
    ),
  );
});

test("daemon boundary guard scopes moved route DTO bans", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api.rs",
    "core/crates/ctx-http/src/api/demo.rs",
    "core/crates/ctx-http/src/api/diagnostics.rs",
    "core/crates/ctx-http/src/api/demo/seed_transcript.rs",
    "core/crates/ctx-http/src/api/health.rs",
    "core/crates/ctx-http/src/api/merge_queue_api.rs",
    "core/crates/ctx-http/src/api/merge_queue_api/logs.rs",
    "core/crates/ctx-http/src/api/mobile_access.rs",
    "core/crates/ctx-http/src/api/telemetry.rs",
    "core/crates/ctx-http/src/api/telemetry/semantic.rs",
    "core/crates/ctx-http/src/api/workspaces.rs",
    "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
  ]) {
    assert.equal(
      apiPatternsForPath(filePath).includes(ROUTE_DTO_SWEEP_API_PATTERNS[0]),
      true,
    );
  }
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/lib_tests/health_diagnostics/mod.rs").includes(
      ROUTE_DTO_SWEEP_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects session artifact route contract leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/artifacts/session/set.rs",
    contents: `
      use ctx_core::ids::{ArtifactId, SessionId};
      use ctx_daemon::daemon::sessions::SessionArtifactInput;
      struct ArtifactInput;
      struct SetSessionArtifactsReq;
      async fn helper(state: SessionsHandle, mcp_auth: McpAuthContext) {
        let session_id = SessionId(uuid::Uuid::parse_str("bad").unwrap());
        let artifact_id = ArtifactId(uuid::Uuid::parse_str("bad").unwrap());
        let _input = SessionArtifactInput {
          absolute_file_path: "/tmp/a".to_string(),
          name: None,
          mime_type: None,
        };
        validate_scoped_mcp_session_context(&state, mcp_auth, session_id).await?;
        state.list_session_artifacts_with_missing_for_route(session_id).await?;
        state.set_session_artifacts_for_route(session_id, vec![]).await?;
        state.open_session_artifact_for_route(session_id, artifact_id).await?;
      }
    `,
    patterns: SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS,
  });

  assert.deepEqual(new Set(violations.map((violation) => violation.name)), new Set([
    "session artifact API imports route contracts from daemon",
    "session artifact API owns local route id parsing",
    "session artifact API owns local set request DTOs",
    "session artifact API constructs raw artifact inputs",
    "session artifact API owns scoped MCP admission",
    "session artifact API calls raw artifact facades",
  ]));
});

test("daemon boundary guard rejects grouped session artifact route contract daemon imports", () => {
  for (const contents of [
    `
      use ctx_daemon::daemon::sessions::{
        SessionArtifactDownloadRouteParams,
        SessionArtifactRouteError,
      };
    `,
    `
      use ctx_daemon::daemon::{
        sessions::{
          SessionArtifactsRouteResponse,
          SetSessionArtifactsRouteRequest,
        },
      };
    `,
  ]) {
    const violations = scanText({
      filePath: "core/crates/ctx-http/src/api/artifacts/download.rs",
      contents,
      patterns: SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS,
    });

    assert(
      violations.some((violation) =>
        violation.name.startsWith("session artifact API imports route contracts from"),
      ),
      `expected grouped route-contract import violation for ${contents}`,
    );
  }
});

test("daemon boundary guard rejects shared session artifact error daemon import", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/artifacts/session.rs",
    contents: `
      use ctx_daemon::daemon::sessions::SessionArtifactRouteError;
      fn status(error: SessionArtifactRouteError) {}
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/artifacts/session.rs"),
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["session artifact API imports route contracts from daemon"],
  );
});

test("daemon boundary guard scopes session artifact route contracts", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/artifacts/session.rs",
    "core/crates/ctx-http/src/api/artifacts/session/list.rs",
    "core/crates/ctx-http/src/api/artifacts/session/set.rs",
    "core/crates/ctx-http/src/api/artifacts/download.rs",
  ]) {
    assert.deepEqual(
      sessionArtifactApiPatternsForPath(filePath),
      SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    sessionArtifactApiPatternsForPath("core/crates/ctx-http/src/api/artifacts/download/response.rs"),
    [],
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/artifacts/session/set.rs",
      contents:
        "state.set_session_artifacts_for_route_params(SessionRouteParams::new(id), SessionArtifactRouteContext::new(ctx), req).await?;",
      patterns: SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects merge queue submit API scoped-admission leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/merge_queue_api/submit.rs",
    contents: `
      use ctx_daemon::daemon::{SessionsHandle, WorkspacesHandle};
      use ctx_daemon::daemon::merge_queue::SubmitMergeQueueEntryRouteRequest;
      use ctx_core::ids::{SessionId, WorktreeId};
      use ctx_merge_queue::MergeQueueSubmitParams;
      async fn helper(sessions: SessionsHandle, workspaces: WorkspacesHandle, mcp_auth: McpAuthContext) {
        let session_id = SessionId(uuid::Uuid::parse_str(raw_session)?);
        let worktree_id = WorktreeId(uuid::Uuid::parse_str(raw_worktree)?);
        if !mcp_auth.allows_merge_queue_submit(session_id, worktree_id) {}
        let _scoped = (mcp_auth.session_id, mcp_auth.worktree_id);
        validate_scoped_mcp_session_context(&sessions, mcp_auth, session_id).await?;
        let params = MergeQueueSubmitParams { session_id: Some(session_id), worktree_id: Some(worktree_id), worktree_root: None, target_branch: None, message: None };
        workspaces.submit_merge_queue_entry(params).await?;
      }
    `,
    patterns: MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "merge queue submit API imports submit route contracts from daemon",
      "merge queue submit API imports sessions handle",
      "merge queue submit API imports sessions handle",
      "merge queue submit API parses session or worktree ids locally",
      "merge queue submit API parses session or worktree ids locally",
      "merge queue submit API parses session or worktree ids locally",
      "merge queue submit API constructs low-level submit params",
      "merge queue submit API constructs low-level submit params",
      "merge queue submit API validates scoped MCP session context",
      "merge queue submit API checks scoped MCP submit capability",
      "merge queue submit API reads scoped MCP ids directly",
      "merge queue submit API calls low-level submit facade",
    ],
  );
});

test("daemon boundary guard catches merge queue submit daemon route-contract imports", () => {
  for (const contents of [
    `
      use ctx_daemon::daemon::merge_queue::{
        MergeQueueSubmitRouteError,
        SubmitMergeQueueEntryRouteRequest,
      };
    `,
    `
      use ctx_daemon::daemon::{
        merge_queue::{
          MergeQueueSubmitRouteErrorKind,
        },
      };
    `,
  ]) {
    const violations = scanText({
      filePath: "core/crates/ctx-http/src/api/merge_queue_api/submit.rs",
      contents,
      patterns: MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS,
    });

    assert(
      violations.some((violation) =>
        violation.name.startsWith(
          "merge queue submit API imports submit route contracts from",
        ),
      ),
      `expected submit route-contract import violation for ${contents}`,
    );
  }
});

test("daemon boundary guard scopes merge queue submit API orchestration roots", () => {
  assert.deepEqual(
    mergeQueueSubmitApiPatternsForPath("core/crates/ctx-http/src/api/merge_queue_api/submit.rs"),
    MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/merge_queue_api/submit.rs").includes(
      MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/merge_queue_api/submit.rs",
      contents: "workspaces.submit_merge_queue_entry_for_route(req, mcp_auth).await?;",
      patterns: MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    mergeQueueSubmitApiPatternsForPath("core/crates/ctx-http/src/api/merge_queue_api/logs.rs"),
    [],
  );
});

test("daemon boundary guard rejects merge queue entry raw route contracts", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/merge_queue_api/actions.rs",
    contents: `
      use ctx_core::models::{MergeQueueEntry as Entry};
      use ctx_merge_queue::MergeQueueSubmitParams;
      use ctx_daemon::daemon::RouteFileDownloadError;
      use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
      struct MergeQueueListParams;
      async fn helper(state: WorkspacesHandle) -> Json<Vec<ctx_core::models::MergeQueueEntry>> {
        let workspace_id = WorkspaceId(uuid::Uuid::parse_str("bad").unwrap());
        let entry_id = MergeQueueEntryId(uuid::Uuid::parse_str("bad").unwrap());
        state.list_merge_queue_entries_for_route(workspace_id, Some(10)).await?;
        state.cancel_merge_queue_entry(workspace_id, entry_id).await?;
        state.retry_merge_queue_entry(workspace_id, entry_id).await?;
        state.download_merge_queue_entry_logs_for_route(workspace_id, entry_id).await?;
        let _ = map_route_file_error(RouteFileDownloadError::NotFound);
        Json(Vec::<Entry>::new())
      }
    `,
    patterns: MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS,
  });

  assert.deepEqual(new Set(violations.map((violation) => violation.name)), new Set([
    "merge queue entry API returns raw entry DTOs",
    "merge queue entry API calls raw action facades",
    "merge queue entry API owns local list request DTOs",
    "merge queue entry API imports low-level merge queue crate",
    "merge queue entry API owns local route id parsing",
    "merge queue entry API calls raw log-download facade",
    "merge queue entry API maps low-level route-file errors",
  ]));
});

test("daemon boundary guard scopes merge queue entry API route contracts", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/merge_queue_api/actions.rs",
    "core/crates/ctx-http/src/api/merge_queue_api/logs.rs",
    "core/crates/ctx-http/src/api/merge_queue_api/submit.rs",
    "core/crates/ctx-http/src/api/merge_queue_api/request.rs",
  ]) {
    assert.deepEqual(
      mergeQueueEntryApiPatternsForPath(filePath),
      MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/merge_queue_api/actions.rs",
      contents:
        "state.cancel_merge_queue_entry_for_route(params).await?; Json::<MergeQueueEntryRouteResponse>(entry)",
      patterns: MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/merge_queue_api/logs.rs",
      contents:
        "state.download_merge_queue_entry_logs_for_route_params(params).await?; let _: MergeQueueEntryRouteParams = params; let _: MergeQueueLogDownloadRouteError = error;",
      patterns: MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS,
    }),
    [],
  );
});

test("appstate guard gates merge queue API route WorkspacesHandle ratchet on capability", () => {
  const contents = `
    use axum::extract::State;
    use ctx_daemon::daemon::WorkspacesHandle;
    async fn route(State(workspaces): State<WorkspacesHandle>) {}
  `;

  assert.deepEqual(
    scanMergeQueueApiHttpRouteRatchet({
      filePath: "core/crates/ctx-http/src/api/merge_queue_api/actions.rs",
      contents,
      mergeQueueApiCapability: false,
    }),
    [],
  );

  const violations = scanMergeQueueApiHttpRouteRatchet({
    filePath: "core/crates/ctx-http/src/api/merge_queue_api/actions.rs",
    contents,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert.equal(
    violations.filter(
      (name) =>
        name
        === "merge queue API route uses broad workspace handle after MergeQueueApiHandle capability",
    ).length,
    2,
  );
});

test("appstate guard rejects merge queue API HTTP raw StoreManager access", () => {
  const violations = scanMergeQueueApiHttpRouteRatchet({
    filePath: "core/crates/ctx-http/src/api/merge_queue_api/logs.rs",
    contents: `
      use ctx_store::{Store, StoreManager};
      async fn route(stores: StoreManager) {
        let _store = daemon.stores().workspace(workspace_id).await?;
      }
    `,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert(
    violations.includes("merge queue API route accesses raw StoreManager"),
  );
});

test("appstate guard rejects merge queue route methods remaining on WorkspacesHandle", () => {
  const violations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/route_contract.rs",
    contents: `
      impl WorkspacesHandle {
        pub async fn list_merge_queue_entry_responses_for_route(&self) {}
        pub async fn cancel_merge_queue_entry_for_route(&self) {}
        pub async fn retry_merge_queue_entry_for_route(&self) {}
        pub async fn download_merge_queue_entry_logs_for_route_params(&self) {}
        pub async fn download_merge_queue_entry_logs_for_route(&self) {}
        pub async fn submit_merge_queue_entry_for_route(&self) {}
      }
    `,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert.equal(
    violations.filter(
      (name) => name === "merge queue API route method remains on WorkspacesHandle",
    ).length,
    6,
  );
});

test("appstate guard rejects broad merge queue API capability fields", () => {
  const violations = scanMergeQueueApiHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct MergeQueueApiHandle {
        state: Arc<DaemonState>,
        daemon: DaemonHandle,
      }

      pub struct MergeQueueRouteHost {
        workspaces: WorkspacesHandle,
        publish_event: SessionEventPublisher,
      }
    `,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert(
    violations.includes("merge queue API capability stores broad handle or daemon state"),
  );
  assert(
    violations.includes(
      "merge queue notice publication effect uses generic session-event name or type",
    ),
  );

  const narrowViolations = scanMergeQueueApiHandleFieldRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      pub struct MergeQueueApiHandle {
        host: Arc<MergeQueueRouteHost>,
      }

      pub struct MergeQueueRouteHost {
        stores: StoreManager,
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        session_stores: SessionStoreLookup,
        merge_queue: Arc<ctx_merge_queue::MergeQueueRuntime>,
        notices: MergeQueueNoticePublisher,
      }
    `,
    mergeQueueApiCapability: true,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard confines merge queue raw StoreManager access to route host trait impl", () => {
  const routeViolations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/route_contract.rs",
    contents: `
      use ctx_store::StoreManager;
      impl MergeQueueApiHandle {
        pub async fn list_merge_queue_entry_responses_for_route(&self) {
          let _store = self.stores.workspace(workspace_id).await?;
        }
      }
    `,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert(
    routeViolations.includes("merge queue route method imports raw StoreManager"),
  );
  assert(
    routeViolations.includes("merge queue API route method accesses raw StoreManager"),
  );

  const hostHelperViolations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/host.rs",
    contents: `
      pub struct MergeQueueRouteHost {
        stores: StoreManager,
      }

      impl MergeQueueRouteHost {
        async fn bad_raw_store_helper(&self, workspace_id: WorkspaceId) -> Result<Store> {
          self.stores.workspace(workspace_id).await
        }
      }
    `,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert(
    hostHelperViolations.includes(
      "merge queue raw StoreManager access outside MergeQueueRouteHost trait implementation",
    ),
  );

  const hostViolations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/host.rs",
    contents: `
      use ctx_store::StoreManager;
      pub struct MergeQueueRouteHost {
        stores: StoreManager,
      }

      impl MergeQueueHost for MergeQueueRouteHost {
        async fn raw_workspace_store(host: &Self, workspace_id: WorkspaceId) -> Result<Store> {
          host.stores.workspace(workspace_id).await
        }
      }
    `,
    mergeQueueApiCapability: true,
  });

  assert.deepEqual(hostViolations, []);
});

test("appstate guard rejects daemon-level scoped MCP helper in merge queue submit code", () => {
  const violations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/submit_route.rs",
    contents: `
      use crate::daemon::{
        require_scoped_mcp_session_context,
        ScopedMcpSessionAccessError,
      };
      impl MergeQueueApiHandle {
        pub async fn submit_merge_queue_entry_for_route(&self) {
          require_scoped_mcp_session_context(self.state.as_ref(), mcp_auth, session_id).await?;
        }
      }
    `,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert(
    violations.includes(
      "merge queue submit code imports daemon-level scoped MCP session helper",
    ),
  );
  assert(
    violations.includes(
      "merge queue submit code calls daemon-level scoped MCP session helper",
    ),
  );

  const narrowViolations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/submit_route.rs",
    contents: `
      impl MergeQueueApiHandle {
        pub async fn submit_merge_queue_entry_for_route(&self) {
          self.session_stores
            .require_scoped_mcp_session_context(mcp_auth, session_id)
            .await?;
          SessionStoreLookup::require_scoped_mcp_session_context(
            &self.session_stores,
            mcp_auth,
            session_id,
          ).await?;
        }
      }
    `,
    mergeQueueApiCapability: true,
  });

  assert.deepEqual(narrowViolations, []);
});

test("appstate guard rejects generic merge queue notice publication effects", () => {
  const violations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/host.rs",
    contents: `
      impl MergeQueueHost for MergeQueueRouteHost {
        async fn publish_notice(host: &Arc<Self>, event: SessionEvent) -> Result<()> {
          host.effects.publish_event(event).await;
          Ok(())
        }
      }
    `,
    mergeQueueApiCapability: true,
  }).map((violation) => violation.name);

  assert.deepEqual(violations, [
    "merge queue notice publication uses generic session-event publisher name",
  ]);

  const narrowViolations = scanMergeQueueApiDaemonImplementationRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/merge_queue/host.rs",
    contents: `
      impl MergeQueueHost for MergeQueueRouteHost {
        async fn publish_notice(host: &Arc<Self>, event: SessionEvent) -> Result<()> {
          host.notices.publish_merge_queue_notice(event).await;
          Ok(())
        }
      }
    `,
    mergeQueueApiCapability: true,
  });

  assert.deepEqual(narrowViolations, []);
});

test("daemon boundary guard rejects terminal REST raw route contracts", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/terminals.rs",
    contents: `
      use ctx_core::ids::{SessionId, TaskId, TerminalId, WorkspaceId, WorktreeId};
      use ctx_core::models::{TerminalSession, TerminalStatus};
      use ctx_daemon::daemon::terminals::CreateTerminalLaunchRequest;
      #[derive(Deserialize)]
      struct CreateTerminalReq;
      #[derive(Serialize)]
      struct TerminalStreamConnectInfo;
      async fn helper(state: TransportHandle) -> Json<Vec<ctx_core::models::TerminalSession>> {
        let _id = TerminalId(uuid::Uuid::parse_str("bad").unwrap());
        state.list_workspace_terminals(workspace_id).await;
        state.create_workspace_terminal(req).await?;
        state.delete_terminal(terminal_id).await;
        state.mint_terminal_stream_token(terminal_id).await;
        Json(Vec::<TerminalSession>::new())
      }
    `,
    patterns: TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS,
  });

  assert.deepEqual(new Set(violations.map((violation) => violation.name)), new Set([
    "terminal REST API exposes raw terminal route DTOs",
    "terminal REST API owns terminal ids or local id parsing",
    "terminal REST API owns local create or stream DTOs",
    "terminal REST API imports low-level launch types",
    "terminal REST API calls raw terminal facades",
  ]));
});

test("daemon boundary guard scopes terminal REST route contracts", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/terminals.rs",
    "core/crates/ctx-http/src/api/terminals/request.rs",
  ]) {
    assert.deepEqual(
      terminalRestRouteApiPatternsForPath(filePath),
      TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/terminals.rs",
      contents: `
        async fn handler(state: TerminalRouteHandle) -> Json<TerminalSessionRouteResponse> {
          state.create_workspace_terminal_for_route(&id, req).await?;
          state.delete_terminal_for_route(DeleteTerminalRouteParams::new(id)).await?;
          state.mint_terminal_stream_token_for_route(MintTerminalStreamTokenRouteParams::new(id)).await?;
          Json(TerminalSessionRouteResponse::fake())
        }
      `,
      patterns: TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    terminalRestRouteApiPatternsForPath("core/crates/ctx-http/src/api/ws/terminal.rs"),
    [],
  );
});

test("daemon boundary guard rejects run archive API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/run_archive.rs",
    contents: `
      mod validation;
      use validation::{parse_archive_run_id, parse_archive_workspace_id, RunArchiveBatchQuery};
      use ctx_core::ids::{RunId, WorkspaceId};
      use ctx_daemon::daemon::workspaces::RunArchiveIngestError;
      struct RunArchiveBatchQuery;
      async fn helper(state: WorkspacesHandle, batch: RunArchiveIngestBatch) {
        let query: RunArchiveBatchQuery = todo!();
        let workspace_id = WorkspaceId(uuid::Uuid::parse_str("bad").unwrap());
        let run_id = RunId(uuid::Uuid::parse_str("bad").unwrap());
        let _ = parse_archive_workspace_id("bad")?;
        let _ = parse_archive_run_id("bad")?;
        let _ = DEFAULT_RUN_ARCHIVE_BATCH_ITEMS;
        let _ = MAX_RUN_ARCHIVE_BATCH_ITEMS;
        let max_items = requested_batch_item_limit(query)?;
        if batch.run.workspace_id != workspace_id {}
        if batch.run.id != run_id {}
        if batch.run.org_id.is_none() || !batch.scope.is_cloud_visible() {}
        let _ = state.build_run_archive_ingest_batch(workspace_id, run_id, max_items).await?;
        let _ = state.acknowledge_run_archive_ingest_batch(workspace_id, run_id, max_items, batch).await?;
        let _ = run_archive_ingest_api_error("build", RunArchiveIngestError::WorkspaceNotFound);
      }
    `,
    patterns: RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "run archive API owns route id parsing",
      "run archive API owns route id parsing",
      "run archive API owns local validation/query helpers",
      "run archive API owns local validation/query helpers",
      "run archive API owns local validation/query helpers",
      "run archive API owns local validation/query helpers",
      "run archive API owns local validation/query helpers",
      "run archive API exposes raw archive body or response models",
      "run archive API references low-level ingest errors",
      "run archive API references low-level ingest errors",
      "run archive API validates acknowledgement batch fields",
      "run archive API validates acknowledgement batch fields",
      "run archive API validates acknowledgement batch fields",
      "run archive API owns batch item limit policy",
      "run archive API owns batch item limit policy",
      "run archive API owns batch item limit policy",
      "run archive API calls low-level archive ingest methods",
      "run archive API calls low-level archive ingest methods",
      "run archive API owns ingest error mapping",
    ],
  );
});

test("daemon boundary guard scopes run archive API orchestration roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/run_archive.rs",
    "core/crates/ctx-http/src/api/run_archive/validation.rs",
  ]) {
    assert.deepEqual(
      runArchiveApiPatternsForPath(filePath),
      RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS[0]),
      true,
    );
  }
  assert.deepEqual(
    runArchiveApiPatternsForPath("core/crates/ctx-http/src/api/updates/check.rs"),
    [],
  );
});

test("daemon boundary guard rejects web-session REST route contract leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/web_sessions/actions.rs",
    contents: `
      use ctx_daemon::daemon::web_sessions::{
        WebSessionAccessError, WebSessionActionError, WebSessionLaunchRequest,
      };
      use ctx_daemon::daemon::{web_sessions::WebSessionAccessError};
      use ctx_daemon::{daemon::web_sessions::WebSessionAccessError};
      use ctx_daemon::{daemon::{web_sessions::WebSessionAccessError, TransportHandle}};
      struct WebSessionListQuery {
        session_id: Option<String>,
      }
      async fn handler(state: TransportHandle, id: String, mut payload: WebSessionRunRequest) {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).unwrap());
        if payload.timeout_ms.is_none() {
          payload.timeout_ms = Some(5 * 60 * 1000);
        }
        payload.timeout_ms = Some(300000);
        let _ = state.list_web_sessions().await;
        let _ = state.get_web_session(&id).await;
        let _ = state.run_web_session(&id, payload).await;
        let _ = state.eval_web_session(&id, WebSessionRunRequest::default()).await;
        let _ = state.close_web_session(&id).await;
      }
    `,
    patterns: [
      ...WEB_SESSION_ACCESS_ERROR_API_PATTERNS,
      ...WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS,
    ],
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "web-session API imports moved access error from daemon",
      "web-session API imports moved access error from daemon",
      "web-session API imports moved access error from daemon",
      "web-session API imports moved access error from daemon",
      "web-session API imports moved access error from daemon",
      "web-session API imports moved access error from daemon",
      "web-session API imports moved access error from daemon",
      "web-session REST API owns session/worktree ids or local id parsing",
      "web-session REST API owns session/worktree ids or local id parsing",
      "web-session REST API exposes old local route DTOs",
      "web-session REST API references low-level route errors",
      "web-session REST API owns run/eval request defaults",
      "web-session REST API owns run/eval request defaults",
      "web-session REST API owns run/eval request defaults",
      "web-session REST API owns run/eval request defaults",
      "web-session REST API owns run/eval request defaults",
      "web-session REST API calls low-level transport facades directly",
      "web-session REST API calls low-level transport facades directly",
      "web-session REST API calls low-level transport facades directly",
      "web-session REST API calls low-level transport facades directly",
      "web-session REST API calls low-level transport facades directly",
    ],
  );
});

test("daemon boundary guard allows web-session signal/view runtime access error", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/web_sessions/stream_view.rs",
    "core/crates/ctx-http/src/api/ws/web_session.rs",
  ]) {
    const violations = scanText({
      filePath,
      contents: `
        use ctx_daemon::daemon::WebSessionRouteHandle;
        use ctx_transport_runtime::web_sessions::WebSessionAccessError;
        fn status(error: WebSessionAccessError) {}
      `,
      patterns: apiPatternsForPath(filePath),
    });

    assert.deepEqual(violations, []);
  }
});

test("daemon boundary guard scopes web-session REST route contract roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/web_sessions/creation.rs",
    "core/crates/ctx-http/src/api/web_sessions/actions.rs",
  ]) {
    assert.deepEqual(
      webSessionRestRouteApiPatternsForPath(filePath),
      WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    webSessionRestRouteApiPatternsForPath("core/crates/ctx-http/src/api/web_sessions/access.rs"),
    [],
  );
  assert.deepEqual(
    webSessionRestRouteApiPatternsForPath(
      "core/crates/ctx-http/src/api/web_sessions/stream_view.rs",
    ),
    [],
  );
  assert.deepEqual(
    webSessionRestRouteApiPatternsForPath("core/crates/ctx-http/src/api/ws/web_session.rs"),
    [],
  );
  for (const filePath of [
    "core/crates/ctx-http/src/api/web_sessions/creation.rs",
    "core/crates/ctx-http/src/api/web_sessions/actions.rs",
    "core/crates/ctx-http/src/api/web_sessions/stream_view.rs",
    "core/crates/ctx-http/src/api/ws/web_session.rs",
  ]) {
    assert.equal(
      apiPatternsForPath(filePath).includes(WEB_SESSION_ACCESS_ERROR_API_PATTERNS[0]),
      true,
    );
  }
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/web_sessions/actions.rs",
      contents: `
        async fn run_web_session(state: WebSessionRouteHandle, id: String, payload: WebSessionActionRouteRequest) {
          let _ = state.run_web_session_for_route(&id, payload).await?;
          let _ = state.close_web_session_for_route(&id).await?;
        }
      `,
      patterns: WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects session head recovery orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
    contents: `
      async fn handler(state: SessionsHandle, min_event_seq: i64) {
        let started_at = Instant::now();
        let workspace_id = state.workspace_id_for_session(session_id).await?;
        if state.is_workspace_deleting(workspace_id).await {}
        if let Some(head) = state.cached_session_head_for_request(session_id, include_events, limit, Some(min_event_seq)).await {
          record_session_head_recovery_metrics(&state, "active_snapshot_cache", "ok", started_at.elapsed(), limit, include_events, Some(&head));
        }
        state.emit_cache_miss("session_head").await;
        let head = state.load_session_head_snapshot_from_store(session_id, limit, include_events).await?;
        if head.last_event_seq < min_event_seq {
          state.emit_cache_rehydrate("session_head", false).await;
        }
        state.update_session_head_cache(head, include_events).await;
      }
    `,
    patterns: SESSION_HEAD_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "session head API owns recovery timing",
      "session head API owns workspace lookup policy",
      "session head API owns workspace lookup policy",
      "session head API owns read-model cache policy",
      "session head API owns read-model cache policy",
      "session head API owns store rebuild policy",
      "session head API owns cache recovery telemetry",
      "session head API owns cache recovery telemetry",
      "session head API owns cache recovery telemetry",
      "session head API owns stale min_event_seq policy",
    ],
  );
});

test("daemon boundary guard scopes session head recovery roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
    "core/crates/ctx-http/src/api/sessions/snapshot/head_metrics.rs",
  ]) {
    assert.deepEqual(
      sessionHeadApiPatternsForPath(filePath),
      SESSION_HEAD_API_ORCHESTRATION_PATTERNS,
    );
  }
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/head.rs").includes(
      SESSION_HEAD_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
      contents: "state.session_head_for_route(req).await?;",
      patterns: SESSION_HEAD_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    sessionHeadApiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/history.rs"),
    [],
  );
});

test("daemon boundary guard rejects session read-model route contracts in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/snapshot/history.rs",
    contents: `
      use ctx_core::models::{SessionSnapshot, SessionHeadSnapshot, SessionHistoryPage, SessionEventsPage, SessionState, SessionTurnTool};
      #[derive(Deserialize)]
      struct SessionHistoryQuery;
      async fn handler(state: SessionsHandle, id: String, turn: String) -> Json<SessionSnapshot> {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        let turn_id = TurnId(uuid::Uuid::parse_str(&turn).unwrap());
        let include_events = parse_boolish_flag(include_events.as_deref())?;
        let include_transient = parse_boolish_flag(include_transient.as_deref())?;
        state.load_session_snapshot(session_id, 60, include_events).await?;
        state.load_session_history_page(session_id, None, 60).await?;
        state.list_session_events_page(session_id, None, 250, None, include_transient).await?;
        state.list_session_turn_tools_for_request(session_id, turn_id).await?;
        state.load_session_state(session_id).await?;
        Json(SessionSnapshot::default())
      }
    `,
    patterns: SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS,
  });

  assert.deepEqual(new Set(violations.map((violation) => violation.name)), new Set([
    "session read-model API exposes raw read-model DTOs",
    "session read-model API owns local route query DTOs",
    "session read-model API owns session or turn id parsing",
    "session read-model API owns boolish flag parsing",
    "session read-model API calls raw read-model facades",
  ]));
});

test("daemon boundary guard scopes session read-model route contracts and allows route DTOs", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/sessions/snapshot.rs",
    "core/crates/ctx-http/src/api/sessions/snapshot/events.rs",
    "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
    "core/crates/ctx-http/src/api/sessions/snapshot/history.rs",
    "core/crates/ctx-http/src/api/sessions/snapshot/state.rs",
  ]) {
    assert.deepEqual(
      sessionReadModelRouteApiPatternsForPath(filePath),
      SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/snapshot/history.rs",
      contents: `
        async fn handler(state: SessionsHandle) -> Json<SessionHistoryRouteResponse> {
          let _ = state.load_session_history_page_for_route(SessionRouteParams::new(id), q).await?;
          let _ = state.list_session_turn_tools_for_route(SessionTurnToolsRouteParams::new(id, turn_id)).await?;
          type Allowed = (
            SessionSnapshotRouteResponse,
            SessionHeadRouteResponse,
            SessionHistoryRouteResponse,
            SessionEventsRouteResponse,
            SessionStateRouteResponse,
            SessionTurnToolsRouteResponse,
          );
          Json(response)
        }
      `,
      patterns: SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    sessionReadModelRouteApiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/vcs.rs"),
    [],
  );
});

test("daemon boundary guard rejects demo seed transcript route contracts in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/demo/seed_transcript.rs",
    contents: `
      use ctx_core::ids::SessionId;
      use ctx_daemon::daemon::sessions::{
        DemoSeedTranscript, DemoSeedTranscriptError, DemoSeedTranscriptTurn,
      };
      async fn handler(state: SessionsHandle, id: String, req: SeedTranscriptReq) {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        if req.turns.is_empty() {
          return Err(StatusCode::BAD_REQUEST);
        }
        let seed = DemoSeedTranscript {
          turns: vec![DemoSeedTranscriptTurn { user, assistant, context_window }],
        };
        state.seed_demo_transcript(session_id, seed).await.map_err(|error| match error {
          DemoSeedTranscriptError::SessionNotFound => StatusCode::NOT_FOUND,
        })?;
        let response = SeedTranscriptResp { seeded_turns: 1 };
      }
    `,
    patterns: DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS,
  });

  assert.deepEqual(
    new Set(violations.map((violation) => violation.name)),
    new Set([
      "demo seed transcript API owns session id parsing",
      "demo seed transcript API owns local route DTOs",
      "demo seed transcript API constructs raw demo seed domain objects",
      "demo seed transcript API owns empty-turn validation",
      "demo seed transcript API calls raw seed transcript facade",
    ]),
  );
});

test("daemon boundary guard scopes demo seed transcript route contracts", () => {
  const filePath = "core/crates/ctx-http/src/api/demo/seed_transcript.rs";
  assert.deepEqual(
    demoSeedTranscriptApiPatternsForPath(filePath),
    DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath(filePath).includes(DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS[0]),
    true,
  );
  assert.deepEqual(
    demoSeedTranscriptApiPatternsForPath("core/crates/ctx-http/src/api/demo.rs"),
    [],
  );
});

test("daemon boundary guard rejects session control route contracts in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/control/ask_user.rs",
    contents: `
      use ctx_daemon::daemon::sessions::ask_user::SubmitAskUserAnswerError;
      use ctx_daemon::daemon::sessions::auth::SessionAuthError;
      use ctx_daemon::daemon::sessions::command_dispatch::SessionSchedulerCommandError;
      use ctx_daemon::daemon::workspaces::{FileCompletionsError, FileCompletionsErrorKind};
      use ctx_providers::ask_user_question::AskUserQuestionOutcome;
      #[derive(Deserialize)]
      struct SubmitAskUserQuestionReq;
      #[derive(Serialize)]
      struct SubmitAskUserQuestionResp;
      async fn handler(state: SessionsHandle, id: String) {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        state.cancel_session(session_id).await?;
        state.interrupt_session(session_id, request_started).await?;
        state.authenticate_session_for_request(session_id, method_id).await?;
        state.submit_ask_user_answer(session_id, submission).await?;
        state.complete_files_for_session(session_id, query, limit).await?;
      }
    `,
    patterns: SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS,
  });

  assert.deepEqual(new Set(violations.map((violation) => violation.name)), new Set([
    "session control API owns session id parsing",
    "session control API owns local route DTOs",
    "session control API imports low-level daemon control errors",
    "session control API calls raw control facades",
  ]));
});

test("daemon boundary guard scopes session control route contracts and allows route DTOs", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/sessions/control/interrupts.rs",
    "core/crates/ctx-http/src/api/sessions/control/authenticate.rs",
    "core/crates/ctx-http/src/api/sessions/control/ask_user.rs",
    "core/crates/ctx-http/src/api/sessions/file_completions.rs",
  ]) {
    assert.deepEqual(
      sessionControlRouteApiPatternsForPath(filePath),
      SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/control/ask_user.rs",
      contents: `
        async fn handler(state: SessionsHandle) -> Json<SubmitAskUserQuestionRouteResponse> {
          let response = state
            .submit_ask_user_question_for_route(SessionRouteParams::new(id), req)
            .await?;
          state.interrupt_session_for_route(SessionRouteParams::new(id), request_started).await?;
          state.complete_files_for_session_for_route(SessionRouteParams::new(id), q).await?;
          Json(response)
        }
        type Allowed = (
          AuthenticateSessionRouteRequest,
          SubmitAskUserQuestionRouteRequest,
          SubmitAskUserQuestionRouteResponse,
          SessionFileCompletionsRouteQuery,
          SessionFileCompletionsRouteResponse,
        );
      `,
      patterns: SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    sessionControlRouteApiPatternsForPath(
      "core/crates/ctx-http/src/api/sessions/messages/post/handler.rs",
    ),
    [],
  );
  assert.deepEqual(
    sessionControlRouteApiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/file_completions.rs"),
    [],
  );
});

test("daemon boundary guard rejects session message command route contracts in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/messages/post/handler.rs",
    contents: `
      use ctx_daemon::daemon::sessions::{PostUserMessageError, PostUserMessageInput, SessionImageBlobStoreError};
      use ctx_daemon::daemon::{PostSessionMessageRouteContext, SessionsHandle};
      use ctx_daemon::daemon::sessions::command_dispatch::SessionSchedulerCommandError;
      use ctx_session_service::message_delivery::MessageClientIdResolutionError;
      #[derive(Deserialize)]
      struct PostMessageReq;
      struct PostMessageParts;
      async fn handler(state: SessionsHandle, id: String, attachment: MessageAttachment) {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        let message_id = MessageId::new();
        let turn_id = TurnId::new();
        let delivery = MessageDelivery::Queued;
        let ids = resolve_message_client_ids(Some(message_id), Some(turn_id))?;
        let enabled = env_bool(std::env::var("CTX_QUEUED_MESSAGES_ENABLED").ok().as_deref());
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
        ensure_image_attachment_mime_type("image/png")?;
        ensure_image_attachment_size(bytes.len())?;
        image_attachment_too_large_error();
        load_image_blob_metadata(&state, blob_id).await?;
        normalize_message_attachments(&state, vec![attachment]).await?;
        state.store_inline_image_blob(&bytes, "image/png", None).await?;
        state.get_blob(blob_id).await?;
        state.post_user_message_for_request(session_id, input).await?;
        state.delete_queued_session_message(session_id, message_id).await?;
      }
    `,
    patterns: SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS,
  });

  assert.deepEqual(new Set(violations.map((violation) => violation.name)), new Set([
    "session message command API owns id parsing",
    "session message command API imports removed post-message context",
    "session message command API owns local route DTOs",
    "session message command API owns delivery or attachment contracts",
    "session message command API owns attachment blob normalization",
    "session message command API owns queued-message env policy",
    "session message command API imports low-level message errors",
    "session message command API calls raw message facades",
  ]));
});

test("daemon boundary guard scopes session message command route contracts and allows route DTOs", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/sessions/mod.rs",
    "core/crates/ctx-http/src/api/sessions/messages.rs",
    "core/crates/ctx-http/src/api/sessions/messages/delete.rs",
    "core/crates/ctx-http/src/api/sessions/messages/post.rs",
    "core/crates/ctx-http/src/api/sessions/messages/post/handler.rs",
    "core/crates/ctx-http/src/api/sessions/messages/attachments/validation.rs",
    "core/crates/ctx-http/src/api/sessions/messages/helpers.rs",
  ]) {
    assert.deepEqual(
      sessionMessageCommandRouteApiPatternsForPath(filePath),
      SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/mod.rs",
      contents: `
        use ctx_daemon::daemon::{PostSessionMessageRouteContext, SessionsHandle};
      `,
      patterns: SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS,
    }).map((violation) => violation.name),
    ["session message command API imports removed post-message context"],
  );

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/messages/post/handler.rs",
      contents: `
        async fn handler(state: SessionsHandle) -> Json<PostSessionMessageRouteResponse> {
          let response = state
            .post_session_message_for_route(
              SessionRouteParams::new(id),
              req,
              run_id_header,
            )
            .await?;
          state
            .delete_session_message_for_route(DeleteSessionMessageRouteParams::new(session_id, id))
            .await?;
          type Allowed = (
            PostSessionMessageRouteRequest,
            PostSessionMessageRouteResponse,
            DeleteSessionMessageRouteParams,
          );
          Json(response)
        }
      `,
      patterns: SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    sessionMessageCommandRouteApiPatternsForPath(
      "core/crates/ctx-http/src/api/sessions/snapshot/history.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects session subagent route contract leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/subagents/handlers.rs",
    contents: `
      use ctx_daemon::daemon::sessions::subagents::{
        AgentSummary, ArchiveAgentReq, ArchiveAgentResp, GetAgentReq, GetAgentResp,
        InterruptAgentReq, InterruptAgentResp, SendInputReq, SendInputResp,
        SpawnAgentReq, SpawnAgentResp, SubagentError, SubagentErrorKind, WaitAgentReq,
        WaitAgentResp,
      };
      use ctx_core::models::{SessionSummary, SubagentInvocation};
      async fn handler(state: SessionsHandle) -> Json<Vec<AgentSummary>> {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        let turn_id = TurnId(uuid::Uuid::parse_str(&turn_id).unwrap());
        let _query = SessionSubagentInvocationsQuery::default();
        let _ = ScopedMcpSessionAccessError::SessionNotFound;
        let _ = logs::redact_sensitive("secret");
        let _ = state.require_scoped_mcp_session_context(mcp_auth, session_id).await;
        let _ = resolve_scoped_parent_session_id(&state, None, id).await;
        let _ = state.spawn_agent(session_id, spawn).await;
        let _ = state.send_input(session_id, send).await;
        let _ = state.archive_agent(session_id, archive).await;
        let _ = state.list_agents(session_id).await;
        let _ = state.get_agent(session_id, get).await;
        let _ = state.interrupt_agent(session_id, interrupt).await;
        let _ = state.wait_agent(session_id, wait).await;
        let _ = state.list_session_subagents_for_request(session_id).await;
        let _ = state.list_session_subagent_invocations_for_request(session_id, Some(turn_id)).await;
        let _ = state.get_session_subagent_invocation_for_request(session_id, "id").await;
        let _ = SessionsHandle::wait_agent(&state, session_id, wait).await;
        let _ = SessionSummary;
        let _ = SubagentInvocation;
        let _ = SubagentErrorKind::BadRequest;
        let _ = SubagentError;
      }
    `,
    patterns: SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS,
  });

  const names = new Set(violations.map((violation) => violation.name));
  assert(names.has("session subagent API owns id parsing"));
  assert(names.has("session subagent API owns local route DTOs"));
  assert(names.has("session subagent API imports low-level subagent errors"));
  assert(names.has("session subagent API imports raw subagent wire DTOs"));
  assert(names.has("session subagent API exposes raw subagent models"));
  assert(names.has("session subagent API redacts scoped errors"));
  assert(names.has("session subagent API validates scoped MCP directly"));
  assert(names.has("session subagent API calls raw subagent facades"));
});

test("daemon boundary guard scopes session subagent route contracts and allows route DTOs", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/subagents.rs").includes(
      SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/subagents/handlers.rs").includes(
      SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/head.rs").includes(
      SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS[0],
    ),
    false,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/subagents/handlers.rs",
      contents: `
        async fn handler(state: SessionsHandle) -> Json<ListAgentsRouteResponse> {
          state
            .list_agents_for_mcp_route(SessionRouteParams::new(id), None)
            .await?;
          state
            .spawn_agent_for_mcp_route(SessionRouteParams::new(id), None, req)
            .await?;
          let _ = SpawnAgentRouteRequest;
          let _ = SessionSubagentRouteErrorKind::BadRequest;
        }
      `,
      patterns: SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects moved subagent route contracts from daemon across HTTP API", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/head.rs").includes(
      SESSION_SUBAGENT_ROUTE_DAEMON_IMPORT_PATTERNS[0],
    ),
    true,
  );

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
    contents: `
      use ctx_daemon::daemon::{
        SpawnAgentRouteRequest,
        sessions::{WaitAgentRouteResponse},
      };
      use ctx_daemon::daemon::sessions::{SessionSubagentRouteError};

      async fn handler() {
        let _ = ctx_daemon::daemon::GetAgentRouteRequest;
        let _ = ctx_daemon::daemon::sessions::ArchiveAgentRouteResponse;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/sessions/snapshot/head.rs"),
  });

  const names = new Set(violations.map((violation) => violation.name));
  assert(names.has("HTTP API imports moved subagent route contracts from daemon"));
  assert(names.has("HTTP API imports moved subagent route contracts from daemon sessions"));
  assert(
    names.has(
      "HTTP API imports moved subagent route contracts from nested daemon sessions group",
    ),
  );
});

test("daemon boundary guard allows subagent route_contract imports only", () => {
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/subagents.rs",
      contents: `
        use ctx_subagent_service::route_contract::{
          SpawnAgentRouteRequest,
          WaitAgentRouteResponse,
        };
      `,
      patterns: apiPatternsForPath("core/crates/ctx-http/src/api/sessions/subagents.rs"),
    }),
    [],
  );

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/subagents.rs",
    contents: `
      use ctx_subagent_service::{SpawnAgentReq};
      async fn handler() {
        let _ = ctx_subagent_service::WaitAgentReq;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/sessions/subagents.rs"),
  });

  const names = violations.map((violation) => violation.name);
  assert(names.includes("HTTP API imports subagent service through root group"));
  assert(names.includes("HTTP API imports non-contract subagent service APIs"));
});

test("daemon boundary guard rejects session VCS API workspace-service orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/snapshot/vcs/diff.rs",
    contents: `
      use ctx_daemon::daemon::sessions::vcs::{
        SessionVcsApplyAction, SessionVcsDiff, SessionVcsDiffQuery, SessionVcsDiffSummary,
        SessionVcsError, SessionVcsGitStatus, SessionVcsGitStatusEntry,
      };
      use ctx_worktree_vcs_service::{
        apply_worktree_vcs_session_patch as apply_patch,
        WorktreeVcsDiffBaseQuery,
      };
      use ctx_worktree_vcs_service as vcs;
      #[derive(Deserialize)]
      struct SessionDiffApplyReq;
      #[derive(Deserialize)]
      struct SessionDiffRouteQuery;
      struct SessionDiffResponse;
      struct SessionDiffSummaryResponse;
      struct SessionGitStatusResponse;
      struct SessionGitStatusEntryResponse;

      async fn handler(sessions: SessionsHandle) {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        let _ = logs::redact_sensitive("secret");
        let _ = SessionVcsApplyAction::Accept;
        let _ = SessionVcsDiffQuery::default();
        let _ = SessionVcsDiff;
        let _ = SessionVcsDiffSummary;
        let _ = SessionVcsError::NotFound;
        let _ = SessionVcsGitStatus;
        let _ = SessionVcsGitStatusEntry;
        let _ = sessions.get_session_vcs_diff_for_request(session_id, query).await;
        let _ = sessions.get_session_vcs_diff_summary_for_request(session_id, query).await;
        let _ = sessions.apply_session_vcs_diff_patch_for_request(session_id, action, patch).await;
        let _ = sessions.get_session_vcs_git_status_for_request(session_id).await;
        let _ = SessionsHandle::get_session_vcs_diff_for_request(&sessions, session_id, query).await;
        let _ = ctx_worktree_vcs_service::worktree_vcs_session_diff_available("x".to_string());
        let _ = session_git_status_summary_from_snapshot(&snapshot);
        let _ = WorktreeDiffBaseResolution;
        let _ = GitStatusEntry;
      }
    `,
    patterns: SESSION_VCS_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name).sort(),
    [
      "session VCS API owns session id parsing",
      "session VCS API owns local route DTOs",
      "session VCS API owns local route DTOs",
      "session VCS API owns local route DTOs",
      "session VCS API owns local route DTOs",
      "session VCS API owns local route DTOs",
      "session VCS API owns local route DTOs",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API imports low-level route contracts",
      "session VCS API redacts low-level route errors",
      "session VCS API calls raw VCS facades",
      "session VCS API calls raw VCS facades",
      "session VCS API calls raw VCS facades",
      "session VCS API calls raw VCS facades",
      "session VCS API calls raw VCS facades",
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
    ].sort(),
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
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/snapshot/vcs/diff.rs",
      contents: `
        async fn handler(state: SessionsHandle) -> Json<SessionVcsDiffRouteResponse> {
          state
            .get_session_vcs_diff_for_route(SessionRouteParams::new(id), query)
            .await?;
          state
            .get_session_vcs_diff_summary_for_route(SessionRouteParams::new(id), query)
            .await?;
          state
            .get_session_vcs_git_status_for_route(SessionRouteParams::new(id))
            .await?;
          state
            .apply_session_vcs_diff_patch_for_route(SessionRouteParams::new(id), req)
            .await?;
          let _ = SessionVcsRouteErrorKind::BadRequest;
        }
      `,
      patterns: SESSION_VCS_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects session model switch orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/titles_and_modes/model.rs",
    contents: `
      use ctx_provider_install::install_state::InstallTarget;
      use ctx_providers::adapters::ProviderAdapter;
      use ctx_session_tools::model_resolution::{compose_model_id, normalize_effort_id, resolve_model_id};
      use ctx_core::models::Session;
      use ctx_daemon::daemon::sessions::{GenerateSessionTitleError, SetSessionModeError, SetSessionModelRequest, SetSessionModelError, SetSessionModelErrorKind};
      #[derive(Deserialize)]
      struct GenerateSessionTitleReq;
      #[derive(Deserialize)]
      struct SetSessionModelReq;
      #[derive(Deserialize)]
      struct SetSessionModeReq;

      async fn handler(sessions: SessionsHandle) -> Json<Session> {
        let session_id = SessionId(uuid::Uuid::parse_str(&id).unwrap());
        let _ = logs::redact_sensitive("secret");
        let _ = GenerateSessionTitleError::Skipped;
        let _ = SetSessionModeError::BadRequest;
        let _ = SetSessionModelRequest { model_id, reasoning_effort };
        let _ = SetSessionModelErrorKind::Forbidden;
        let _ = SetSessionModelError;
        sessions.generate_session_title_for_request(session_id, None, None).await?;
        sessions.set_session_model_for_request(session_id, request).await?;
        sessions.set_session_mode_for_request(session_id, mode_id).await?;
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
    violations.map((violation) => violation.name).sort(),
    [
      "session title/model/mode API owns session id parsing",
      "session title/model/mode API exposes raw Session success shape",
      "session title/model/mode API exposes raw Session success shape",
      "session title/model/mode API imports low-level route errors",
      "session title/model/mode API imports low-level route errors",
      "session title/model/mode API imports low-level route errors",
      "session title/model/mode API imports low-level route errors",
      "session title/model/mode API imports low-level route errors",
      "session title/model/mode API imports low-level route errors",
      "session title/model/mode API owns local route DTOs",
      "session title/model/mode API owns local route DTOs",
      "session title/model/mode API owns local route DTOs",
      "session title/model/mode API redacts low-level route errors",
      "session title/model/mode API calls raw title/model/mode facades",
      "session title/model/mode API calls raw title/model/mode facades",
      "session title/model/mode API calls raw title/model/mode facades",
      "session model API imports model-resolution helpers directly",
      "session model API imports provider adapter directly",
      "session model API imports provider install target directly",
      "session model API imports provider install target directly",
      "session model API loads target parts directly",
      "session model API ensures provider adapter directly",
      "session model API loads provider model catalog directly",
      "session model API persists model update directly",
      "session model API defines old orchestration helpers",
      "session model API defines old orchestration helpers",
      "session model API defines old orchestration helpers",
      "session model API defines old orchestration helpers",
    ].sort(),
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
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/sessions/titles_and_modes/model.rs",
      contents: `
        async fn handler(state: SessionsHandle) -> Json<SetSessionModelRouteResponse> {
          let response = state
            .set_session_model_for_route(SessionRouteParams::new(id), req)
            .await?;
          state
            .generate_session_title_for_route(SessionRouteParams::new(id), title_req)
            .await?;
          state
            .set_session_mode_for_route(SessionRouteParams::new(id), mode_req)
            .await?;
          type Allowed = (
            GenerateSessionTitleRouteRequest,
            GenerateSessionTitleRouteResponse,
            SetSessionModelRouteRequest,
            SetSessionModelRouteResponse,
            SetSessionModeRouteRequest,
            SessionTitleModelModeRouteError,
            SessionTitleModelModeRouteErrorKind,
          );
          Json(response)
        }
      `,
      patterns: SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects provider bootstrap orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/bootstrap.rs",
    contents: `
      async fn handler(providers: ProvidersHandle) {
        let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id)?);
        let _ = ProvidersBootstrapErrorKind::NotFound;
        let _ = ProvidersBootstrapError::internal("boom");
        let body = serde_json::json!({ "error": "boom" });
        workspace_providers_bootstrap(&state, workspace_id).await?;
        providers.workspace_providers_bootstrap(workspace_id).await?;
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
      "provider bootstrap API parses workspace ids directly",
      "provider bootstrap API matches bootstrap errors directly",
      "provider bootstrap API matches bootstrap errors directly",
      "provider bootstrap API owns bootstrap error JSON",
      "provider bootstrap API calls broad bootstrap facade",
      "provider bootstrap API calls broad bootstrap facade",
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
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/bootstrap.rs",
      contents: `
        providers.workspace_providers_bootstrap_for_route(request).await?;
        let request = ProvidersBootstrapRouteRequest { workspace_id };
        let _ = ProvidersBootstrapRouteErrorKind::BadRequest;
      `,
      patterns: PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects provider harness config API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/harness_config.rs",
    contents: `
      use ctx_harness_sources as harness_sources;
      async fn handler(providers: ProvidersHandle) {
        let req = SelectHarnessSourceReq { source_kind: HarnessSourceKind::Endpoint, endpoint_id: None };
        providers.get_provider_harness_config(&id).await?;
        providers.select_provider_harness_source(&id, req.source_kind, req.endpoint_id).await?;
        provider_harness_bad_request_error(err);
        let _ = logs::redact_sensitive(&err.to_string());
      }
    `,
    patterns: PROVIDER_HARNESS_CONFIG_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider harness config API imports harness source domain",
      "provider harness config API owns select request DTO",
      "provider harness config API references source kind directly",
      "provider harness config API calls broad get/select facades",
      "provider harness config API calls broad get/select facades",
      "provider harness config API owns bad-request mapping",
      "provider harness config API redacts lower-level errors",
    ],
  );
});

test("daemon boundary guard scopes provider harness config API roots", () => {
  assert.deepEqual(
    providerHarnessConfigApiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/harness_config.rs",
    ),
    PROVIDER_HARNESS_CONFIG_API_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/harness_config.rs").includes(
      PROVIDER_HARNESS_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/harness_config.rs",
      contents: `
        providers.get_provider_harness_config_for_route(&id).await?;
        providers.select_provider_harness_source_for_route(&id, req).await?;
        providers.upsert_provider_harness_endpoint_for_route(&id, req).await?;
      `,
      patterns: PROVIDER_HARNESS_CONFIG_API_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    providerHarnessConfigApiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects provider harness endpoint API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
    contents: `
      use ctx_harness_sources as harness_sources;
      async fn handler(providers: ProvidersHandle) {
        let endpoint = HarnessEndpointUpsert { name, endpoint_id };
        let config = providers.upsert_provider_harness_endpoint(&id, endpoint, manual_model_ids).await?;
        providers.refresh_provider_harness_endpoint_models(&id, &endpoint_id).await?;
        providers.set_provider_harness_endpoint_manual_models(&id, &endpoint_id, model_ids).await?;
        providers.delete_provider_harness_endpoint(&id, &endpoint_id).await?;
        let status = if error.contains("unknown endpoint") { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST };
        let error = logs::redact_sensitive(&err.to_string());
        provider_harness_delete_error(err);
      }
    `,
    patterns: PROVIDER_HARNESS_ENDPOINT_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider harness endpoint API imports harness source domain",
      "provider harness endpoint API constructs low-level endpoint upsert",
      "provider harness endpoint API owns delete error taxonomy",
      "provider harness endpoint API owns delete error taxonomy",
      "provider harness endpoint API redacts lower-level errors",
      "provider harness endpoint API calls low-level endpoint facades",
      "provider harness endpoint API calls low-level endpoint facades",
      "provider harness endpoint API calls low-level endpoint facades",
      "provider harness endpoint API calls low-level endpoint facades",
    ],
  );
});

test("daemon boundary guard scopes provider harness endpoint API roots", () => {
  assert.deepEqual(
    providerHarnessEndpointApiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
    ),
    PROVIDER_HARNESS_ENDPOINT_API_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
    ).includes(PROVIDER_HARNESS_ENDPOINT_API_PATTERNS[0]),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
      contents: "providers.upsert_provider_harness_endpoint_for_route(&id, req).await?;",
      patterns: PROVIDER_HARNESS_ENDPOINT_API_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    providerHarnessEndpointApiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/harness_config.rs",
    ),
    [],
  );
});

test("daemon boundary guard rejects provider install API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/installs/status.rs",
    contents: `
      use ctx_provider_install::install_state::{InstallId, InstallInfo, InstallProgressEvent, InstallTarget};
      async fn handler(providers: ProvidersHandle) {
        let target = parse_provider_install_target(raw)?;
        let _query = InstallTargetQuery { target: None };
        let _response = InstallStartResponse { provider_id, install_id, target };
        let _req = GetInstallStatusesReq { install_ids };
        let _item = InstallStatusBatchItem { install_id, info };
        let _resp = GetInstallStatusesResp { installs };
        let install_id = uuid::Uuid::parse_str(raw)?;
        providers.start_provider_install(&provider_id, target).await?;
        providers.start_all_provider_installs(target).await?;
        providers.get_provider_install_info(install_id).await;
        providers.cancel_provider_install(install_id).await;
        providers.list_provider_install_events(install_id).await;
        providers.provider_install_event_sender(install_id).await;
      }
    `,
    patterns: PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider install API imports install domain directly",
      "provider install API parses install target directly",
      "provider install API owns old install route DTOs",
      "provider install API owns old install route DTOs",
      "provider install API owns old install route DTOs",
      "provider install API owns old install route DTOs",
      "provider install API owns old install route DTOs",
      "provider install API references low-level install domain types",
      "provider install API calls low-level install facades",
      "provider install API calls low-level install facades",
      "provider install API calls low-level install facades",
      "provider install API calls low-level install facades",
      "provider install API calls low-level install facades",
      "provider install API calls low-level install facades",
      "provider install API parses install ids directly",
    ],
  );
});

test("daemon boundary guard scopes provider install API roots", () => {
  assert.deepEqual(
    providerInstallApiPatternsForPath("core/crates/ctx-http/src/api/provider_launch/errors.rs"),
    PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  );
  assert.deepEqual(
    providerInstallApiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/installs/status.rs",
    ),
    PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/installs/start.rs",
    ).includes(PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS[0]),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/installs/start.rs",
      contents: "providers.start_provider_install_for_route(&id, target).await?;",
      patterns: PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    providerInstallApiPatternsForPath("core/crates/ctx-http/src/api/providers/status/routes.rs"),
    [],
  );
});

test("daemon boundary guard catches provider install daemon route-contract imports", () => {
  const direct = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch/errors.rs",
    contents: `
      use ctx_daemon::daemon::providers::ProviderInstallJsonRouteError;
    `,
    patterns: PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  });
  assert.deepEqual(
    direct.map((violation) => violation.name),
    ["provider install API imports install route contracts from daemon"],
  );

  const multiline = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch/errors.rs",
    contents: `
      use ctx_daemon::daemon::providers::{
        ProviderInstallJsonRouteError,
        ProviderInstallJsonRouteErrorStatus,
      };
    `,
    patterns: PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  });
  assert.deepEqual(
    multiline.map((violation) => violation.name),
    ["provider install API imports install route contracts from daemon"],
  );

  const nested = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch/errors.rs",
    contents: `
      use ctx_daemon::daemon::{
        providers::{ProviderInstallInfo, ProviderInstallStatusesRouteRequest},
        ProvidersHandle,
      };
    `,
    patterns: PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  });
  assert.deepEqual(
    nested.map((violation) => violation.name),
    [
      "provider install API imports install route contracts from nested daemon providers group",
    ],
  );
});

test("daemon boundary guard rejects provider admin API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/install.rs",
    contents: `
      struct MatrixRefreshResponse;
      struct DevRestartProvidersReq;
      struct DevRestartProvidersResp;
      struct DevRestartProvidersResult;

      fn dev_tools_enabled() -> bool {
        std::env::var("CTX_DEV_MODE")
          .ok()
          .and_then(ctx_core::boolish::parse_boolish)
          .unwrap_or(false)
      }

      fn parse_restart_mode(value: &str) -> Option<ProviderRestartMode> {
        None
      }

      async fn handler(providers: ProvidersHandle, result: RestartResult) {
        let summary = providers.refresh_provider_inventory().await?;
        let results = providers.restart_all_provider_adapters("dev", mode).await;
        let message = format!("failed to refresh provider statuses: {err:#}");
        let _mapped = (result.provider_id, result.status, result.message);
      }
    `,
    patterns: PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider admin API owns matrix refresh DTOs",
      "provider admin API owns dev restart DTOs",
      "provider admin API owns dev restart DTOs",
      "provider admin API owns dev restart DTOs",
      "provider admin API reads dev-mode env directly",
      "provider admin API reads dev-mode env directly",
      "provider admin API reads dev-mode env directly",
      "provider admin API parses restart modes directly",
      "provider admin API calls low-level admin facades",
      "provider admin API calls low-level admin facades",
      "provider admin API owns matrix refresh error mapping",
      "provider admin API maps restart results locally",
    ],
  );
});

test("daemon boundary guard scopes provider admin API roots", () => {
  assert.deepEqual(
    providerAdminApiPatternsForPath("core/crates/ctx-http/src/api/providers/install.rs"),
    PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/install.rs").includes(
      PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/types/dev.rs").includes(
      PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS[1],
    ),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/install.rs",
      contents: `
        providers.refresh_provider_matrix_for_route().await?;
        providers.dev_restart_providers_for_route(req).await?;
        let _err: Option<ProviderAdminRouteError> = None;
        let _kind = ProviderAdminRouteErrorKind::BadRequest;
        let _req: Option<ProviderDevRestartRouteRequest> = None;
        let _resp: Option<ProviderDevRestartRouteResponse> = None;
        let _matrix: Option<ProviderMatrixRefreshRouteResponse> = None;
      `,
      patterns: PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    providerAdminApiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/installs/start.rs",
    ),
    [],
  );
});

test("daemon boundary guard catches moved provider route-contract daemon imports", () => {
  const parentPath = "core/crates/ctx-http/src/api/providers.rs";
  const unrelatedApiPath = "core/crates/ctx-http/src/api/sessions/example.rs";

  const direct = scanText({
    filePath: parentPath,
    contents: `
      use ctx_daemon::daemon::providers::ProvidersBootstrapRouteRequest;
    `,
    patterns: apiPatternsForPath(parentPath),
  });
  assert.deepEqual(
    direct.map((violation) => violation.name),
    ["provider API imports moved provider route contracts from daemon"],
  );

  const multiline = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
    contents: `
      use ctx_daemon::daemon::providers::{
        ProviderHarnessEndpointRouteError,
        UpsertProviderHarnessEndpointRouteRequest,
      };
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
    ),
  });
  assert.deepEqual(
    multiline.map((violation) => violation.name),
    ["provider API imports moved provider route contracts from daemon"],
  );

  const nested = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/install.rs",
    contents: `
      use ctx_daemon::daemon::{
        providers::{ProviderAdminRouteError, ProviderDevRestartRouteRequest},
        ProvidersHandle,
      };
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/providers/install.rs"),
  });
  assert.deepEqual(
    nested.map((violation) => violation.name),
    [
      "provider API imports moved provider route contracts from nested daemon providers group",
    ],
  );

  const runtimeImport = scanText({
    filePath: parentPath,
    contents: `
      use ctx_provider_runtime::{
        ProviderAdminRouteError,
        ProvidersBootstrapRouteRequest,
        SelectProviderHarnessSourceRouteRequest,
      };
    `,
    patterns: apiPatternsForPath(parentPath),
  });
  assert.deepEqual(runtimeImport, []);

  const unrelatedApiImport = scanText({
    filePath: unrelatedApiPath,
    contents: `
      use ctx_daemon::daemon::providers::ProviderMatrixRefreshRouteResponse;
    `,
    patterns: apiPatternsForPath(unrelatedApiPath),
  });
  assert.deepEqual(
    unrelatedApiImport.map((violation) => violation.name),
    ["provider API imports moved provider route contracts from daemon"],
  );
});

test("daemon boundary guard rejects provider launch auth API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/auth.rs",
    contents: `
      use ctx_daemon::daemon::providers::{AuthenticateProviderForWorkspaceRouteRequest, ProviderAuthCheckRouteResponse};
      async fn handler(providers: ProvidersHandle) {
        let req = AuthenticateProviderReq { method_id: None };
        let response = ProviderAuthCheckResp::from(snapshot);
        let _snapshot: ProviderAuthCheckSnapshot = snapshot;
        let _ = ProviderAuthCheckError::WorkspaceNotFound;
        let workspace_id = parse_workspace_id(&ws_id)?;
        authenticate_provider_for_workspace_runtime(state, workspace, workspace_id, &provider_id, target, None).await?;
        providers.authenticate_provider_for_workspace(workspace_id, &provider_id, req.method_id).await?;
        providers.verify_provider_for_workspace(workspace_id, &provider_id).await?;
        provider_auth_check_error_json(err);
        workspace_execution_settings_error_json(&err);
        provider_launch_config_error_response(err);
      }
    `,
    patterns: PROVIDER_LAUNCH_AUTH_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider launch auth API imports auth route contracts from daemon",
      "provider launch auth API calls provider-runtime auth orchestration directly",
      "provider launch auth API owns auth response DTO",
      "provider launch auth API references auth check snapshot directly",
      "provider launch auth API matches auth check errors directly",
      "provider launch auth API owns auth request DTO",
      "provider launch auth API calls broad auth/verify facades",
      "provider launch auth API calls broad auth/verify facades",
      "provider launch auth API owns auth error mapping",
      "provider launch auth API parses workspace id directly",
      "provider launch auth API uses shared route error helpers directly",
      "provider launch auth API uses shared route error helpers directly",
    ],
  );
});

test("daemon boundary guard scopes provider launch auth API roots", () => {
  assert.deepEqual(
    providerLaunchAuthApiPatternsForPath("core/crates/ctx-http/src/api/provider_launch.rs"),
    PROVIDER_LAUNCH_AUTH_API_PATTERNS,
  );
  assert.deepEqual(
    providerLaunchAuthApiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/auth.rs",
    ),
    PROVIDER_LAUNCH_AUTH_API_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/auth/verify.rs",
    ).includes(PROVIDER_LAUNCH_AUTH_API_PATTERNS[0]),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/auth.rs",
      contents: `
        use ctx_provider_runtime::{
          AuthenticateProviderForWorkspaceRouteRequest,
          ProviderAuthCheckRouteError,
          ProviderAuthCheckRouteResponse,
        };
        providers.authenticate_provider_for_workspace_for_route(req).await?;
        providers.verify_provider_for_workspace_for_route(req).await?;
      `,
      patterns: PROVIDER_LAUNCH_AUTH_API_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    providerLaunchAuthApiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/options.rs",
    ),
    [],
  );
});

test("daemon boundary guard catches multiline provider launch auth daemon imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch.rs",
    contents: `
      use ctx_daemon::daemon::providers::{
        ProviderInstallInfo,
        AuthenticateProviderForWorkspaceRouteRequest,
      };
    `,
    patterns: PROVIDER_LAUNCH_AUTH_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["provider launch auth API imports auth route contracts from daemon"],
  );
});

test("daemon boundary guard catches nested provider launch auth daemon imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch.rs",
    contents: `
      use ctx_daemon::daemon::{
        providers::{ProviderInstallInfo, AuthenticateProviderForWorkspaceRouteRequest},
        ProvidersHandle,
      };
    `,
    patterns: PROVIDER_LAUNCH_AUTH_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider launch auth API imports auth route contracts from nested daemon providers group",
    ],
  );
});

test("daemon boundary guard rejects provider launch options API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/options.rs",
    contents: `
      use ctx_daemon::daemon::providers::{ProviderOptionsRouteError, ProviderOptionsRouteRequest};
      async fn handler(providers: ProvidersHandle) {
        let _ = ProviderOptionsResponseError::WorkspaceNotFound;
        prepare_provider_options_response(state, request).await?;
        providers.get_provider_options_response(workspace_id, &provider_id).await?;
        get_provider_options_response(&state, workspace_id, &provider_id).await?;
        ctx_daemon::daemon::providers::get_provider_options_response(&state, workspace_id, &provider_id).await?;
        let workspace_id = parse_workspace_id(&ws_id)?;
        let parsed = uuid::Uuid::parse_str(&ws_id)?;
        provider_options_response_error_json(err);
        workspace_execution_settings_error_json(&err);
        provider_launch_config_error_response(err);
        logs::redact_sensitive(&err.to_string());
      }
    `,
    patterns: PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider launch options API imports options route contracts from daemon",
      "provider launch options API calls provider-runtime options orchestration directly",
      "provider launch options API matches options errors directly",
      "provider launch options API calls broad options method facade",
      "provider launch options API calls options free function directly",
      "provider launch options API calls options free function directly",
      "provider launch options API parses workspace id directly",
      "provider launch options API parses UUIDs directly",
      "provider launch options API owns options error mapping",
      "provider launch options API uses shared route error helpers directly",
      "provider launch options API uses shared route error helpers directly",
      "provider launch options API redacts errors directly",
    ],
  );
});

test("daemon boundary guard scopes provider launch options API roots", () => {
  assert.deepEqual(
    providerLaunchOptionsApiPatternsForPath("core/crates/ctx-http/src/api/provider_launch.rs"),
    PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
  );
  assert.deepEqual(
    providerLaunchOptionsApiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/options.rs",
    ),
    PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/options/extra.rs",
    ).includes(PROVIDER_LAUNCH_OPTIONS_API_PATTERNS[0]),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/provider_launch/handlers/options.rs",
      contents: `
        use ctx_provider_runtime::{ProviderOptionsRouteError, ProviderOptionsRouteRequest};
        providers.get_provider_options_for_route(req).await?;
      `,
      patterns: PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    providerLaunchOptionsApiPatternsForPath(
      "core/crates/ctx-http/src/api/provider_launch/handlers/auth.rs",
    ),
    [],
  );
});

test("daemon boundary guard catches multiline provider launch options daemon imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch.rs",
    contents: `
      use ctx_daemon::daemon::providers::{
        ProviderInstallInfo,
        ProviderOptionsRouteRequest,
      };
    `,
    patterns: PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["provider launch options API imports options route contracts from daemon"],
  );
});

test("daemon boundary guard catches nested provider launch options daemon imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/provider_launch.rs",
    contents: `
      use ctx_daemon::daemon::{
        providers::{ProviderInstallInfo, ProviderOptionsRouteRequest},
        ProvidersHandle,
      };
    `,
    patterns: PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider launch options API imports options route contracts from nested daemon providers group",
    ],
  );
});

test("daemon boundary guard rejects provider auth-import API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/imports.rs",
    contents: `
      use ctx_daemon::daemon::providers::{
        ProviderAuthImportCandidatesRouteResponse,
        ProviderAuthImportRouteRequest,
      };
      async fn handler(providers: ProvidersHandle) {
        let _ = ProviderAuthImportCandidate { id };
        let _ = ctx_provider_auth_import::list_provider_auth_import_candidates().await?;
        let _ = ProviderAuthImportCandidatesResponse { candidates };
        let _ = ProviderAuthImportProfilesResponse { profiles };
        let _ = ProviderAuthImportReq { candidate_ids };
        let _ = ProviderAuthImportResponse { results };
        let candidates = ctx_daemon::daemon::providers::list_provider_auth_import_candidates().await?;
        providers.list_provider_auth_import_profiles().await?;
        providers.import_provider_auth_candidates(candidate_ids).await?;
        let _ = logs::redact_sensitive(&e.to_string());
        let _ = err.to_string();
      }
    `,
    patterns: PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider auth import API imports route contracts from daemon",
      "provider auth import API imports auth-import domain payloads directly",
      "provider auth import API calls auth-import domain crate directly",
      "provider auth import API owns old route DTOs",
      "provider auth import API owns old route DTOs",
      "provider auth import API owns old route DTOs",
      "provider auth import API owns old route DTOs",
      "provider auth import API calls candidates free function directly",
      "provider auth import API calls broad auth-import facades",
      "provider auth import API calls broad auth-import facades",
      "provider auth import API redacts errors locally",
      "provider auth import API stringifies lower-level errors locally",
      "provider auth import API stringifies lower-level errors locally",
    ],
  );
});

test("daemon boundary guard rejects moved provider helper imports from daemon", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/tests/install_policy.rs",
    contents: `
      use ctx_daemon::daemon::providers::provider_auth_import_result_requires_restart;
      use ctx_daemon::daemon::providers::{
        ProviderLoginRuntimeCommand,
        resolve_claude_login_runtime_from_config,
      };
      use ctx_daemon::daemon::{
        providers::{resolve_cursor_login_runtime_from_config},
      };
    `,
    patterns: PROVIDER_TEST_HELPER_DAEMON_IMPORT_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider API imports moved provider test helpers from daemon",
      "provider API imports moved provider test helpers from daemon",
      "provider API imports moved provider test helpers from nested daemon providers group",
    ],
  );
});

test("daemon boundary guard scopes moved provider helper imports to provider API tests", () => {
  assert.equal(
    providerTestHelperDaemonImportPatternsForPath(
      "core/crates/ctx-http/src/api/providers/tests/install_policy.rs",
    ).includes(PROVIDER_TEST_HELPER_DAEMON_IMPORT_PATTERNS[0]),
    true,
  );
  assert.equal(
    providerTestHelperDaemonImportPatternsForPath(
      "core/crates/ctx-http/src/api/providers/login/claude/runtime.rs",
    ).includes(PROVIDER_TEST_HELPER_DAEMON_IMPORT_PATTERNS[0]),
    true,
  );
  assert.deepEqual(
    providerTestHelperDaemonImportPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces.rs",
    ),
    [],
  );
});

test("daemon boundary guard scopes provider auth-import API roots", () => {
  assert.deepEqual(
    providerAuthImportApiPatternsForPath("core/crates/ctx-http/src/api/providers.rs"),
    PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS,
  );
  assert.deepEqual(
    providerAuthImportApiPatternsForPath("core/crates/ctx-http/src/api/providers/imports.rs"),
    PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/imports.rs").includes(
      PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers.rs",
      contents: `
        use ctx_daemon::daemon::providers::{
          ProviderAuthImportCandidatesRouteResponse,
          ProviderAuthImportRouteRequest,
        };
      `,
      patterns: apiPatternsForPath("core/crates/ctx-http/src/api/providers.rs"),
    }).map((violation) => violation.name),
    ["provider auth import API imports route contracts from daemon"],
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/imports.rs",
      contents: `
        use ctx_provider_auth_import::{
          ProviderAuthImportCandidatesRouteResponse,
          ProviderAuthImportProfilesRouteResponse,
          ProviderAuthImportRouteError,
          ProviderAuthImportRouteRequest,
          ProviderAuthImportRouteResponse,
        };
        providers.import_provider_auth_candidates_for_route(req).await?;
      `,
      patterns: PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    providerAuthImportApiPatternsForPath("core/crates/ctx-http/src/api/providers/accounts.rs"),
    [],
  );
});

test("daemon boundary guard rejects provider status API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/status/routes.rs",
    contents: `
      async fn handler(providers: ProvidersHandle, query: InstallTargetQuery) {
        let target = parse_provider_install_target(query.target.as_deref())?;
        let _ = ProviderStatusResponseError::NotFound { provider_id };
        providers.providers_statuses_response(target, false).await;
        providers.provider_status_response(&id, target).await?;
        provider_status_response_error(err);
      }
    `,
    patterns: PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider status API parses install target directly",
      "provider status API owns install target query DTO",
      "provider status API matches provider status errors directly",
      "provider status API calls broad status facades",
      "provider status API calls broad status facades",
      "provider status API owns provider status error mapping",
    ],
  );
});

test("daemon boundary guard scopes provider status API roots", () => {
  assert.deepEqual(
    providerStatusApiPatternsForPath("core/crates/ctx-http/src/api/providers.rs"),
    PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
  );
  assert.deepEqual(
    providerStatusApiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/status/routes.rs",
    ),
    PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/status/usage.rs").includes(
      PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/types/queries.rs",
    ).includes(PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS[1]),
    true,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/status/routes.rs",
      contents: `
        providers.providers_statuses_for_route(query).await?;
        providers.provider_status_for_route(&id, query).await?;
      `,
      patterns: PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard allows runtime provider status route contracts only", () => {
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers.rs",
      contents: `
        use ctx_provider_runtime::{
          ProviderStatusListRouteError, ProviderStatusRouteError, ProviderStatusRouteErrorKind,
          ProviderStatusRouteQuery,
        };
      `,
      patterns: PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers.rs",
    contents: `
      use ctx_daemon::daemon::providers::{ProviderStatusRouteQuery};
      use ctx_provider_runtime::provider_status_service;

      async fn handler() {
        provider_status_service::provider_status_response(state, "codex", target).await?;
      }
    `,
    patterns: PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider status API imports route contracts from daemon",
      "provider status API imports provider-runtime status orchestration",
      "provider status API calls broad status facades",
    ],
  );
});

test("daemon boundary guard rejects provider usage API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/accounts/codex/usage.rs",
    contents: `
      use ctx_provider_runtime::provider_usage;

      struct ProviderUsageQuery;
      struct CodexAccountsUsageResponse;
      struct CodexAccountUsageEntry;

      async fn handler(providers: ProvidersHandle, entry: CodexAccountUsageRecord) {
        let _snapshot: provider_usage::ProviderUsageSnapshot = providers
          .load_provider_usage("codex", true)
          .await?;
        let _records = providers.load_codex_accounts_usage(false).await?;
        let _entry = CodexAccountUsageEntry {
          account_id: entry.account_id,
          usage: entry.usage,
        };
        provider_usage_internal_error(err);
      }
    `,
    patterns: PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider usage API imports provider-runtime usage DTOs",
      "provider usage API imports provider-runtime usage DTOs",
      "provider usage API owns route query DTO",
      "provider usage API owns Codex account usage DTOs",
      "provider usage API owns Codex account usage DTOs",
      "provider usage API owns Codex account usage DTOs",
      "provider usage API calls low-level usage facades",
      "provider usage API calls low-level usage facades",
      "provider usage API owns usage internal-error mapping",
      "provider usage API maps account usage records locally",
      "provider usage API maps account usage records locally",
      "provider usage API maps account usage records locally",
    ],
  );
});

test("daemon boundary guard scopes provider usage API roots", () => {
  assert.deepEqual(
    providerUsageApiPatternsForPath("core/crates/ctx-http/src/api/providers.rs"),
    PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
  );
  assert.deepEqual(
    providerUsageApiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/status/usage.rs",
    ),
    PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/accounts/codex/usage.rs",
    ).includes(PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/types/accounts/responses.rs",
    ).includes(PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS[2]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/accounts/codex.rs").includes(
      PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/status/usage.rs",
      contents: `
        async fn handler(providers: ProvidersHandle, query: ProviderUsageRouteQuery) {
          let _snapshot: Option<ProviderUsageRouteSnapshot> = None;
          let _accounts: Option<CodexAccountsUsageRouteResponse> = None;
          let _error: Option<ProviderUsageRouteError> = None;
          providers.provider_usage_for_route("codex", query).await?;
          providers.codex_accounts_usage_for_route(ProviderUsageRouteQuery::default()).await?;
        }
      `,
      patterns: PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard allows runtime provider usage route contracts only", () => {
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers.rs",
      contents: `
        use ctx_provider_runtime::{
          CodexAccountsUsageRouteResponse, ProviderUsageRouteError, ProviderUsageRouteQuery,
          ProviderUsageRouteSnapshot,
        };
      `,
      patterns: PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers.rs",
    contents: `
      use ctx_daemon::daemon::providers::{ProviderUsageRouteQuery};
      use ctx_provider_runtime::provider_usage;

      async fn handler() {
        provider_usage::refresh_provider_usage_for(state, "codex", env).await?;
      }
    `,
    patterns: PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider usage API imports route contracts from daemon",
      "provider usage API imports provider-runtime usage DTOs",
      "provider usage API calls provider-runtime usage orchestration",
    ],
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
        let _request: Option<CodexActiveAccountReq> = None;
        providers.codex_accounts_response().await?;
        providers.import_host_codex_auth_response(label).await?;
        let _probe: Option<provider_accounts::CodexHostImportProbe> = None;
        ctx_daemon::daemon::providers::probe_host_codex_auth_candidate().await;
      }

      pub(crate) async fn amp_accounts_response(providers: &ProvidersHandle) {}
      pub(crate) struct GeminiAccountUpsertReq { oauth_creds_json: String }
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
      "provider account API owns account request DTOs",
      "provider account API owns account request DTOs",
      "provider account API calls broad account response facades",
      "provider account API calls broad account response facades",
      "provider account API calls low-level Codex host import probe",
      "provider account API calls low-level Codex host import probe",
    ],
  );
});

test("daemon boundary guard scopes provider account orchestration patterns", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers.rs").includes(
      PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
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
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/providers/types/accounts/requests.rs",
    ).includes(PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS[7]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/bootstrap.rs").includes(
      PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard allows provider-account route contracts only", () => {
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/accounts.rs",
      contents: `
        pub(crate) use ctx_provider_accounts::route_contract::{
          AmpAccountUpsertRouteRequest, AmpAccountsResponse, ProviderAccountRouteError,
          ProviderAccountRouteErrorKind, ProviderActiveAccountRouteRequest,
        };
      `,
      patterns: PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers.rs",
    contents: `
      use ctx_daemon::daemon::providers::{ProviderActiveAccountRouteRequest};
      use ctx_provider_accounts as provider_accounts;

      async fn handler() {
        let _ = provider_accounts::load_codex_registry(data_root).await?;
      }
    `,
    patterns: PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider account API imports route contracts from daemon",
      "provider account API imports provider-account domain directly",
    ],
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
        providers.start_gemini_browser_login(label).await;
        providers.gemini_login_status(login_id).await;
        providers.start_kimi_oauth_login(label).await?;
        let _req = GeminiLoginStartReq { label };
        let _resp = KimiLoginStartResp { login_id, auth_url, device_code };
        let _ = "login not found";
        err.route_safe_message();
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
      "managed browser login API calls low-level start/status facades",
      "managed browser login API calls low-level start/status facades",
      "managed browser login API calls low-level start/status facades",
      "managed browser login API owns login start DTOs",
      "managed browser login API owns login start DTOs",
      "managed browser login API owns login not-found mapping",
      "managed browser login API maps Kimi start errors directly",
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
    false,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
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
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/codex.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/cursor_login.rs").includes(
      MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
      contents: `
        let req = ProviderLoginStartRouteRequest::default();
        providers.start_gemini_login_for_route(req).await;
        providers.gemini_login_status_for_route(&id).await?;
      `,
      patterns: MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects provider-account login status DTOs in HTTP login routes", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
    contents: `
      use ctx_provider_accounts::GeminiLoginStatus;
      use ctx_provider_accounts::{AmpLoginStatus, QwenLoginStatus};
      type GeminiStatus = provider_accounts::GeminiLoginStatus;
      async fn get_status() -> Result<Json<GeminiLoginStatus>, StatusCode> {}
      let _cursor: Option<ctx_provider_accounts::CursorLoginStatus> = None;
      let _codex: Option<provider_accounts::CodexLoginStatus> = None;
    `,
    patterns: PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "provider login API exposes provider-account login status DTOs",
      "provider login API exposes provider-account login status DTOs",
      "provider login API exposes provider-account login status DTOs",
      "provider login API exposes provider-account login status DTOs",
      "provider login API exposes provider-account login status DTOs",
      "provider login API exposes provider-account login status DTOs",
    ],
  );
});

test("daemon boundary guard scopes provider login status DTO bans", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/browser/gemini.rs").includes(
      PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/claude/session.rs").includes(
      PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/cursor_login.rs").includes(
      PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/accounts/codex.rs").includes(
      PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS[0],
    ),
    false,
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
      contents: `
        let _status: Option<GeminiLoginStatusRouteResponse> = None;
        providers.gemini_login_status_for_route(&id).await?;
      `,
      patterns: PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard catches provider login daemon route-contract imports", () => {
  const direct = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
    contents: `
      use ctx_daemon::daemon::providers::ProviderLoginStartRouteRequest;
    `,
    patterns: PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS,
  });
  assert.deepEqual(
    direct.map((violation) => violation.name),
    ["provider login API imports login route contracts from daemon"],
  );

  const multiline = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
    contents: `
      use ctx_daemon::daemon::providers::{
        CursorLoginStatusRouteResponse,
        ProviderLoginRouteErrorKind,
      };
    `,
    patterns: PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS,
  });
  assert.deepEqual(
    multiline.map((violation) => violation.name),
    ["provider login API imports login route contracts from daemon"],
  );

  const nested = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
    contents: `
      use ctx_daemon::daemon::{
        providers::{CodexLoginStartRouteResponse, ClaudeLoginRouteError},
        ProvidersHandle,
      };
    `,
    patterns: PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS,
  });
  assert.deepEqual(
    nested.map((violation) => violation.name),
    ["provider login API imports login route contracts from nested daemon providers group"],
  );
});

test("daemon boundary guard rejects provider-account prelude leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers.rs",
    contents: `
      use ctx_provider_accounts as provider_accounts;
    `,
    patterns: PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["provider API prelude exposes provider-account module"],
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers.rs").includes(
      PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS[0],
    ),
    true,
  );
});

test("daemon boundary guard catches provider prelude daemon login route-contract imports", () => {
  const direct = scanText({
    filePath: "core/crates/ctx-http/src/api/providers.rs",
    contents: `
      use ctx_daemon::daemon::providers::CodexLoginCompleteRouteRequest;
    `,
    patterns: PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS,
  });
  assert.deepEqual(
    direct.map((violation) => violation.name),
    ["provider API prelude imports login route contracts from daemon"],
  );

  const multiline = scanText({
    filePath: "core/crates/ctx-http/src/api/providers.rs",
    contents: `
      use ctx_daemon::daemon::providers::{
        ClaudeLoginStartRouteResponse,
        ProviderLoginStartRouteResponse,
      };
    `,
    patterns: PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS,
  });
  assert.deepEqual(
    multiline.map((violation) => violation.name),
    ["provider API prelude imports login route contracts from daemon"],
  );

  const nested = scanText({
    filePath: "core/crates/ctx-http/src/api/providers.rs",
    contents: `
      use ctx_daemon::daemon::{
        providers::{AmpLoginStatusRouteResponse, CursorLoginRouteErrorKind},
        ProvidersHandle,
      };
    `,
    patterns: PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS,
  });
  assert.deepEqual(
    nested.map((violation) => violation.name),
    [
      "provider API prelude imports login route contracts from nested daemon providers group",
    ],
  );
});

test("daemon boundary guard keeps provider login daemon import bans narrow", () => {
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers.rs",
      contents: `
        use ctx_daemon::daemon::ProvidersHandle;
      `,
      patterns: PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS,
    }),
    [],
  );

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/login/claude.rs",
      contents: stripCfgTestItems(`
        #[cfg(test)]
        pub(crate) async fn resolve_claude_login_runtime_from_config(
          data_root: &std::path::Path,
        ) -> anyhow::Result<ctx_daemon::daemon::providers::ProviderLoginRuntimeCommand> {
          runtime::resolve_claude_login_runtime_from_config(data_root).await
        }
      `),
      patterns: apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/claude.rs"),
    }),
    [],
  );
});

test("daemon boundary guard rejects raw workspace route DTOs and attachment orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/attachment_routes.rs",
    contents: `
      use ctx_core::models::{
        Workspace,
        Worktree,
        WorkspaceAttachment,
        WorkspaceActiveSnapshot,
        WorkspaceActiveHeadBatch,
      };
      use ctx_workspace_container::WorkspaceContainerStatus;
      type Container = Option<WorkspaceContainerStatus>;
      async fn handler() -> Result<Json<Vec<WorkspaceAttachment>>, StatusCode> {
        let _snapshot: Option<ctx_core::models::WorkspaceActiveSnapshot> = None;
        let _cfg: Option<AttachmentConfig> = None;
        require_workspace_ctx(&workspaces, &id).await?;
        workspaces.get_workspace(workspace_id).await?;
        workspaces.upsert_workspace_attachment(workspace_id, cfg).await?;
        workspaces.delete_workspace_attachment(workspace_id, kind, name).await?;
        workspaces.sync_workspace_attachments(&workspace, true).await?;
      }
    `,
    patterns: WORKSPACE_ROUTE_CONTRACT_API_PATTERNS,
  });

  const names = violations.map((violation) => violation.name);
  assert(names.includes("workspace route API exposes raw workspace route DTOs"));
  assert(names.includes("workspace attachment API owns attachment config construction"));
  assert(names.includes("workspace attachment API loads workspace context in HTTP"));
  assert(names.includes("workspace attachment API calls raw attachment facade methods"));
});

test("daemon boundary guard rejects workspace REST route identity and error mapping leaks", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
    contents: `
      use ctx_core::ids::{WorkspaceId, WorktreeId};
      use ctx_daemon::daemon::RouteFileDownloadError;
      use ctx_daemon::daemon::workspaces::{
        FileCompletionsError,
        FileCompletionsErrorKind,
        WorkspaceDeleteError,
        WorkspaceHarnessContainerError,
        WorkspaceHydrationError,
        WorkspaceHydrationErrorKind,
      };
      mod context;
      use context::*;
      async fn handler(workspaces: WorkspacesHandle) {
        let _ = parse_workspace_id(&id)?;
        let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id)?);
        let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id)?);
        workspaces.load_workspace_active_snapshot_for_route(workspace_id).await?;
        workspaces.load_workspace_active_heads_for_route(workspace_id).await?;
        workspaces.get_worktree_for_route(worktree_id).await?;
        workspaces.download_worktree_bootstrap_logs_for_route(worktree_id).await?;
        workspaces.workspace_harness_container_status_for_route(workspace_id).await?;
        workspaces.stop_workspace_harness_container(workspace_id).await?;
        WorkspacesHandle::ensure_workspace_harness_container(&workspaces, workspace_id).await?;
        workspaces.delete_workspace(workspace_id).await?;
        workspaces.complete_files_for_workspace(workspace_id, None, None).await?;
        let _ = logs::redact_sensitive("secret");
        let _ = map_effective_execution_settings_error(err);
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/harness_container.rs"),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "workspace REST route API parses route ids directly",
    "workspace REST route API uses local workspace context helper",
    "workspace REST route API inspects low-level workspace errors",
    "workspace REST route API calls low-level workspace facades directly",
    "workspace harness-container API maps execution settings locally",
    "workspace harness-container API redacts low-level errors locally",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }

  const fileCompletionViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/file_completions.rs",
    contents: `
      use ctx_daemon::daemon::workspaces::{FileCompletionsError, FileCompletionsErrorKind};
      async fn handler(workspaces: WorkspacesHandle) {
        workspaces.complete_files_for_workspace(workspace_id, None, None).await?;
        let _ = map_file_completions_error(error);
      }
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/file_completions.rs",
    ),
  });
  const fileCompletionNames = new Set(
    fileCompletionViolations.map((violation) => violation.name),
  );
  assert(
    fileCompletionNames.has("workspace REST route API inspects low-level workspace errors"),
  );
  assert(
    fileCompletionNames.has("workspace REST route API calls low-level workspace facades directly"),
  );
  assert(
    fileCompletionNames.has(
      "workspace file-completion API maps low-level completion errors locally",
    ),
  );

  const managementViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      async fn handler(workspaces: WorkspacesHandle) {
        workspaces.workspace_merge_queue_config_for_route(workspace_id).await?;
        workspaces.update_workspace_merge_queue_config_for_route(workspace_id, request).await?;
        workspaces.workspace_primary_branch_for_request(workspace_id).await?;
        workspaces.update_workspace_primary_branch_for_request(workspace_id, request).await?;
        workspaces.workspace_execution_config_for_request(workspace_id).await?;
        workspaces.update_workspace_execution_config_for_request(workspace_id, request).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management.rs"),
  });
  assert(
    managementViolations
      .map((violation) => violation.name)
      .includes("workspace REST route API calls low-level workspace facades directly"),
  );

  const attachmentViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/attachment_routes.rs",
    contents: `
      async fn handler(workspaces: WorkspacesHandle) {
        workspaces.list_workspace_attachments_for_route(workspace_id).await?;
        workspaces.sync_workspace_attachments_for_route(workspace_id, request).await?;
        workspaces.create_and_sync_workspace_attachment_for_route(workspace_id, request).await?;
        workspaces.delete_and_sync_workspace_attachment_for_route(workspace_id, request).await?;
      }
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/attachment_routes.rs",
    ),
  });
  assert(
    attachmentViolations
      .map((violation) => violation.name)
      .includes("workspace REST route API calls low-level workspace facades directly"),
  );

  const routeViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
    contents: `
      async fn get_workspace_harness_container(
          State(workspaces): State<WorkspacesHandle>,
          Path(id): Path<String>,
      ) -> Result<Json<Option<WorkspaceHarnessContainerStatusRouteResponse>>, StatusCode> {
          workspaces
              .workspace_harness_container_status_for_route_params(WorkspaceRouteParams::new(id))
              .await?;
          Ok(Json(None))
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/harness_container.rs"),
  });
  assert.deepEqual(routeViolations, []);
});

test("daemon boundary guard scopes workspace route contract bans", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/attachments.rs").includes(
      WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management.rs").includes(
      WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/attachment_routes.rs",
    ).includes(WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
    ).includes(WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0]),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/registry/delete.rs").includes(
      WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/file_completions.rs").includes(
      WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/registry/future/nested.rs").includes(
      WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/context.rs").includes(
      WORKSPACE_ROUTE_CONTRACT_API_PATTERNS[0],
    ),
    true,
  );
  assert(
    scanText({
      filePath: "core/crates/ctx-http/src/api/workspaces/context.rs",
      contents: `
        fn parse_workspace_id(id: &str) -> Result<WorkspaceId, StatusCode> {
          Ok(WorkspaceId(uuid::Uuid::parse_str(id)?))
        }
      `,
      patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/context.rs"),
    })
      .map((violation) => violation.name)
      .includes("workspace REST route API uses local workspace context helper"),
  );
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/workspaces/attachments.rs",
      contents: `
        async fn list() -> Result<Json<Vec<WorkspaceAttachmentRouteResponse>>, StatusCode> {
          workspaces
            .list_workspace_attachments_for_route_params(WorkspaceRouteParams::new(id))
            .await?;
          workspaces
            .sync_workspace_attachments_for_route_params(WorkspaceRouteParams::new(id), request)
            .await?;
        }
      `,
      patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/attachments.rs"),
    }),
    [],
  );
});

test("daemon boundary guard rejects Cursor process login orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/cursor_login/session.rs",
    contents: `
      struct CursorLoginStartReq;
      struct CursorLoginStartResp;

      mod session;

      async fn handler(providers: ProvidersHandle) {
        providers.start_cursor_process_login(label).await?;
        start_cursor_process_login(state, label).await?;
        providers.cursor_login_status(login_id).await;
        cursor_login_status(state, login_id).await;
        let _kind = CursorProcessLoginStartErrorKind::RuntimeCommandBadRequest;
        let _err: CursorProcessLoginStartError = err;
        let _ = err.route_safe_message();
        let _ = "login not found";
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
      "Cursor process login API owns route DTOs",
      "Cursor process login API owns route DTOs",
      "Cursor process login API calls low-level route facades",
      "Cursor process login API calls low-level route facades",
      "Cursor process login API calls low-level route facades",
      "Cursor process login API calls low-level route facades",
      "Cursor process login API matches route errors directly",
      "Cursor process login API matches route errors directly",
      "Cursor process login API matches route errors directly",
      "Cursor process login API owns login not-found mapping",
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

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/cursor_login.rs",
      contents: `
        async fn handler(providers: ProvidersHandle, req: CursorLoginStartRouteRequest) {
          let _ = providers.start_cursor_login_for_route(req).await;
          let _ = providers.cursor_login_status_for_route("login-id").await;
          let _kind = CursorLoginRouteErrorKind::NotFound;
          let _err: Option<CursorLoginRouteError> = None;
          let _resp: Option<CursorLoginStartRouteResponse> = None;
          let _status: Option<provider_accounts::CursorLoginStatus> = None;
        }
      `,
      patterns: CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS,
    }),
    [],
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
        let _start = CodexLoginStartReq { label };
        let _complete = CodexLoginCompleteResp { accepted: true, status_code: 200 };
        providers.start_codex_app_server_login(label).await?;
        providers.codex_login_status(account_id).await;
        providers.complete_codex_app_server_login(account_id, callback_url, token).await?;
        start_codex_app_server_login(&state, label).await?;
        codex_login_status(&state, account_id).await;
        complete_codex_app_server_login(&state, account_id, callback_url, token).await?;
        let _ = CodexLoginCompleteErrorKind::BadRequest;
        let _ = CodexLoginCompleteError::new(kind, message);
        let _ = CodexLoginStartError::from_error(err);
        err.route_safe_message();
        let _ = "login not found";
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
      "Codex app-server login API owns route DTOs",
      "Codex app-server login API owns route DTOs",
      "Codex app-server login API calls low-level route facades",
      "Codex app-server login API calls low-level route facades",
      "Codex app-server login API calls low-level route facades",
      "Codex app-server login API calls low-level route facades",
      "Codex app-server login API calls low-level route facades",
      "Codex app-server login API calls low-level route facades",
      "Codex app-server login API matches route errors directly",
      "Codex app-server login API matches route errors directly",
      "Codex app-server login API matches route errors directly",
      "Codex app-server login API matches route errors directly",
      "Codex app-server login API owns login not-found mapping",
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
    false,
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
  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/login/codex.rs",
      contents: `
        let req = CodexLoginStartRouteRequest::default();
        providers.start_codex_login_for_route(req).await?;
        providers.codex_login_status_for_route(&id).await?;
        providers.complete_codex_login_for_route(&id, complete).await?;
        let _ = CodexLoginRouteErrorKind::BadRequest;
      `,
      patterns: CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS,
    }),
    [],
  );
});

test("daemon boundary guard rejects Claude setup-token login orchestration in HTTP", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/providers/login/claude/session/process.rs",
    contents: `
      struct ClaudeLoginStartReq;
      struct ClaudeLoginStartResp;

      mod auth_url;
      mod process;
      mod setup_token;

      async fn handler(providers: ProvidersHandle) {
        providers.start_claude_setup_token_login(label).await?;
        start_claude_setup_token_login(state, label).await?;
        providers.claude_login_status(login_id).await;
        claude_login_status(state, login_id).await;
        let _kind = ClaudeSetupTokenLoginStartErrorKind::BadRequest;
        let _err: ClaudeSetupTokenLoginStartError = err;
        let _ = err.route_safe_message();
        let _ = "login not found";
        tokio::spawn(async move {});
        let mut cmd = tokio::process::Command::new("claude");
        let _pty = NativePtySystem::default();
        let _ = CommandBuilder::new("claude");
        let _ = providers.resolve_claude_login_runtime().await?;
        spawn_claude_setup_token_command(runtime)?;
        start_claude_login_process(runtime).await?;
        monitor_claude_login(providers.clone(), login_id, label, login).await;
        wait_for_claude_login_observation(child, deadline).await?;
        finalize_claude_login(providers, login_id, label, setup_token).await;
        let _ = extract_claude_setup_token("ok");
        let _ = CLAUDE_BROWSER_OPEN_MARKER;
        let _ = ClaudeAuthUrlSource::Transcript;
        let _ = normalize_claude_login_line(line);
        read_trailing_claude_login_lines(&mut rx, wait).await;
        let _ = claude_browser_open_shim_script(true);
        let _ = CLAUDE_BROWSER_AUTH_TIER;
        let _ = CTX_CLAUDE_AUTH_URL_CAPTURE_PATH;
        let _ = CLAUDE_LOGIN_URL_WAIT;
        for key in DAEMON_AUTH_ENV_VARS {}
        providers.start_claude_login_session(auth_url).await;
        providers.set_claude_login_auth_url(login_id, auth_url).await;
        providers.finish_claude_login_session(login_id, status, account_id, error, auth_url).await;
        providers.add_claude_account_for_login(label, setup_token).await?;
      }
    `,
    patterns: CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "Claude setup-token login API owns route DTOs",
      "Claude setup-token login API owns route DTOs",
      "Claude setup-token login API calls low-level route facades",
      "Claude setup-token login API calls low-level route facades",
      "Claude setup-token login API calls low-level route facades",
      "Claude setup-token login API calls low-level route facades",
      "Claude setup-token login API matches route errors directly",
      "Claude setup-token login API matches route errors directly",
      "Claude setup-token login API matches route errors directly",
      "Claude setup-token login API owns login not-found mapping",
      "Claude setup-token login API owns monitor task spawning",
      "Claude setup-token login API declares setup-token implementation modules",
      "Claude setup-token login API declares setup-token implementation modules",
      "Claude setup-token login API declares setup-token implementation modules",
      "Claude setup-token login API owns process spawning",
      "Claude setup-token login API owns process spawning",
      "Claude setup-token login API owns process spawning",
      "Claude setup-token login API resolves runtime directly",
      "Claude setup-token login API owns process lifecycle",
      "Claude setup-token login API owns process lifecycle",
      "Claude setup-token login API owns process lifecycle",
      "Claude setup-token login API owns process lifecycle",
      "Claude setup-token login API owns process lifecycle",
      "Claude setup-token login API owns auth-url parsing",
      "Claude setup-token login API owns auth-url parsing",
      "Claude setup-token login API owns auth-url parsing",
      "Claude setup-token login API owns auth-url parsing",
      "Claude setup-token login API owns auth-url parsing",
      "Claude setup-token login API owns browser-open shim",
      "Claude setup-token login API owns browser-open shim",
      "Claude setup-token login API owns browser-open shim",
      "Claude setup-token login API owns auth runtime env",
      "Claude setup-token login API owns auth runtime env",
      "Claude setup-token login API mutates login sessions directly",
      "Claude setup-token login API mutates login sessions directly",
      "Claude setup-token login API mutates login sessions directly",
      "Claude setup-token login API finalizes Claude accounts directly",
    ],
  );
});

test("daemon boundary guard scopes Claude setup-token login orchestration patterns", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login.rs").includes(
      CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/auth_url/extract.rs").includes(
      CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/claude/session.rs").includes(
      CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/providers/login/codex.rs").includes(
      CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/providers/login/claude/session.rs",
      contents: `
        async fn handler(providers: ProvidersHandle, req: ClaudeLoginStartRouteRequest) {
          let _ = providers.start_claude_login_for_route(req).await;
          let _ = providers.claude_login_status_for_route("login-id").await;
          let _kind = ClaudeLoginRouteErrorKind::NotFound;
          let _err: Option<ClaudeLoginRouteError> = None;
          let _resp: Option<ClaudeLoginStartRouteResponse> = None;
          let _status: Option<provider_accounts::ClaudeLoginStatus> = None;
        }
      `,
      patterns: CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS,
    }),
    [],
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

test("daemon boundary guard rejects task route raw contracts", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/tasks/handlers/listing.rs",
    contents: `
      use ctx_core::models::{Task, Session, WorkspaceArchivedPage, WorkspaceTaskSummary, WorkspaceIndexCursor};
      use ctx_daemon::daemon::tasks::{CreateTaskInput, CreateTaskSessionInput, TaskCreateError, TaskSessionCreateError, TaskLifecycleError};
      #[derive(Deserialize)]
      struct WorkspaceArchivedQuery;
      #[derive(Serialize)]
      struct ArchiveTaskResponse;
      async fn handler(tasks: TasksHandle) -> Json<Vec<Task>> {
        let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id).unwrap());
        let sort_at = DateTime::parse_from_rfc3339(raw).unwrap();
        tasks.list_workspace_tasks(workspace_id).await?;
        tasks.list_workspace_archived_page(workspace_id, cursor, 50).await?;
        tasks.list_task_sessions(task_id).await?;
        tasks.mark_task_read(task_id).await?;
        tasks.mark_task_unread(task_id).await?;
        tasks.update_task_title(task_id, title).await?;
        tasks.archive_task(task_id).await?;
        tasks.unarchive_task(task_id).await?;
        tasks.delete_task(task_id).await?;
        tasks.create_task_for_workspace(workspace_id, input).await?;
        tasks.create_session_for_task(task_id, input).await?;
        Json(Vec::<Task>::new())
      }
    `,
    patterns: TASK_ROUTE_API_CONTRACT_PATTERNS,
  });

  assert.deepEqual(new Set(violations.map((violation) => violation.name)), new Set([
    "task route API imports raw task/session/archive models",
    "task route API returns raw task/session DTOs",
    "task route API owns task/workspace id or cursor parsing",
    "task route API owns local task route DTOs",
    "task route API imports low-level task inputs or errors",
    "task route API calls raw task handle facades",
  ]));
});

test("daemon boundary guard scopes task route contracts and allows route DTO names", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/tasks.rs",
    "core/crates/ctx-http/src/api/tasks/creation_task.rs",
    "core/crates/ctx-http/src/api/tasks/creation_session/create.rs",
    "core/crates/ctx-http/src/api/tasks/handlers/listing.rs",
    "core/crates/ctx-http/src/api/tasks/task_title.rs",
    "core/crates/ctx-http/src/api/tasks/task_deletion.rs",
  ]) {
    assert.deepEqual(
      taskRouteApiPatternsForPath(filePath),
      TASK_ROUTE_API_CONTRACT_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(TASK_ROUTE_API_CONTRACT_PATTERNS[0]),
      true,
    );
  }

  assert.deepEqual(
    scanText({
      filePath: "core/crates/ctx-http/src/api/tasks/handlers/read_state.rs",
      contents: `
        async fn handler(tasks: TasksHandle) -> Json<TaskRouteResponse> {
          let task = tasks.mark_task_read_for_route(TaskRouteParams::new(id)).await?;
          tasks.update_task_title_for_route(TaskRouteParams::new(id), req).await?;
          tasks.archive_task_for_route(TaskRouteParams::new(id)).await?;
          tasks.delete_task_for_route(TaskRouteParams::new(id)).await?;
          Json(task)
        }
        type Allowed = (TaskRouteResponse, SessionRouteResponse, WorkspaceArchivedPageRouteResponse);
      `,
      patterns: TASK_ROUTE_API_CONTRACT_PATTERNS,
    }),
    [],
  );
  assert.deepEqual(
    taskRouteApiPatternsForPath("core/crates/ctx-http/src/api/sessions/example.rs"),
    [],
  );
});

test("daemon boundary guard rejects settings API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/settings.rs",
    contents: `
      use ctx_settings_service::HostExecutionPolicy;
      async fn update_settings(State(state): State<CoreHandle>, Json(req): Json<UpdateSettingsReq>) {
        let current = state.load_settings().await?;
        let policy = HostExecutionPolicy::current()?;
        policy.validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)?;
        let next = ctx_settings_service::apply_update(current, req);
        state.save_settings(&next).await?;
        state.apply_settings_side_effects(&next).await;
        state.public_settings_for_response(&next).await;
      }
    `,
    patterns: SETTINGS_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "settings API imports settings service directly",
      "settings API imports settings service directly",
      "settings API owns host execution policy checks",
      "settings API owns host execution policy checks",
      "settings API owns host execution policy checks",
      "settings API applies settings updates directly",
      "settings API owns settings persistence sequencing",
      "settings API owns settings persistence sequencing",
      "settings API owns settings persistence sequencing",
      "settings API owns settings persistence sequencing",
    ],
  );
});

test("daemon boundary guard allows settings API route-contract imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/settings.rs",
    contents: `
      use ctx_settings_service::route_contract::{
        SettingsRouteError,
        SettingsRouteErrorKind,
      };
      fn status(error: SettingsRouteError) -> StatusCode {
        match error.kind() {
          SettingsRouteErrorKind::Forbidden => StatusCode::FORBIDDEN,
          SettingsRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
      }
    `,
    patterns: SETTINGS_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard rejects title-generation daemon status leaves", () => {
  const rejectedImports = [
    `
      use ctx_daemon::daemon::sessions::title_generation::{
        TitleGenerationLocalModelStatus,
        TitleGenerationLocalRuntimeStatus,
      };
    `,
    `
      use ctx_daemon::{daemon::sessions::title_generation::TitleGenerationLocalRuntimeStatus};
    `,
    `
      use ctx_daemon::daemon::sessions::title_generation as daemon_title_generation;
      type RuntimeStatus = daemon_title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::daemon::sessions::{title_generation as daemon_title_generation};
      type RuntimeStatus = daemon_title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::{daemon::sessions::{title_generation as daemon_title_generation}};
      type RuntimeStatus = daemon_title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::daemon::sessions::*;
      type RuntimeStatus = title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::{daemon::sessions::*};
      type RuntimeStatus = title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::daemon::sessions as daemon_sessions;
      type RuntimeStatus = daemon_sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::{daemon::{sessions as daemon_sessions}};
      type RuntimeStatus = daemon_sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::daemon as daemon_alias;
      type RuntimeStatus = daemon_alias::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::{daemon as daemon_alias};
      type RuntimeStatus = daemon_alias::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon as ctxd;
      type RuntimeStatus = ctxd::daemon::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::{self as ctxd};
      type RuntimeStatus = ctxd::daemon::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      extern crate ctx_daemon as ctxd;
      type RuntimeStatus = ctxd::daemon::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ctx_daemon::{daemon::{self}};
      type RuntimeStatus = daemon::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      type RuntimeStatus =
        ctx_daemon::daemon::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
    `
      use ::ctx_daemon as ctxd;
      type RuntimeStatus = ctxd::daemon::sessions::title_generation::TitleGenerationLocalRuntimeStatus;
    `,
  ];

  for (const contents of rejectedImports) {
    const violations = scanText({
      filePath: "core/crates/ctx-http/src/api/title_generation.rs",
      contents,
      patterns: TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
    });

    assert.ok(
      violations.some(
        (violation) =>
          violation.name ===
          "title-generation API imports daemon beyond title-generation local handle",
      ),
      `expected title-generation daemon import violation for:\n${contents}`,
    );
  }

  const allowed = scanText({
    filePath: "core/crates/ctx-http/src/api/title_generation.rs",
    contents: "use ctx_daemon::daemon::TitleGenerationLocalHandle;",
    patterns: TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
  });

  assert.deepEqual(allowed, []);

  const sessionTitleTestViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
    contents: `
      use ctx_daemon::daemon::sessions::title_generation::{
        generate_title_for_prompt,
        TitleGenerationSource,
      };
    `,
    patterns: TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
  });

  assert.ok(
    sessionTitleTestViolations.some(
      (violation) =>
        violation.name ===
        "title-generation API imports daemon beyond title-generation local handle",
    ),
    "expected title-generation session API test to reject daemon title policy imports",
  );

  const sessionTitleServiceAllowed = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
    contents: `
      use ctx_session_title_service::title_generation::{
        generate_title_for_prompt,
        TitleGenerationSource,
      };
    `,
    patterns: TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
  });

  assert.deepEqual(sessionTitleServiceAllowed, []);
});

test("daemon boundary guard rejects CLI init daemon workspace bootstrap", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/main.rs",
    contents: `
      async fn run_init(root: Option<String>) -> anyhow::Result<()> {
        ctx_daemon::daemon::init_workspace(root).await?;
        Ok(())
      }
    `,
    patterns: CTX_HTTP_MAIN_DAEMON_INIT_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["ctx CLI init calls daemon workspace init"],
  );
});

test("daemon boundary guard rejects telemetry export filesystem pathing", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/telemetry.rs",
    contents: `
      async fn export_telemetry(State(core): State<CoreHandle>) {
        let path = ctx_observability::perf_telemetry::perf_log_path_for_date(core.data_root(), &date);
      }
    `,
    patterns: TELEMETRY_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "telemetry API derives perf log path directly",
      "telemetry API accesses daemon data root directly",
    ],
  );
});

test("daemon boundary guard scopes settings, title-generation, and telemetry API orchestration roots", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/settings.rs").includes(
      SETTINGS_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/title_generation.rs").includes(
      TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/mod.rs").includes(
      TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath(
      "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
    ).includes(TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS[0]),
    true,
  );
  assert.deepEqual(
    titleGenerationApiPatternsForPath("core/crates/ctx-http/src/api/title_generation.rs"),
    TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
  );
  assert.deepEqual(
    titleGenerationApiPatternsForPath("core/crates/ctx-http/src/api/mod.rs"),
    TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
  );
  assert.deepEqual(
    titleGenerationApiPatternsForPath(
      "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
    ),
    TITLE_GENERATION_API_ROUTE_CONTRACT_PATTERNS,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/telemetry.rs").includes(
      TELEMETRY_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/updates/check.rs").includes(
      SETTINGS_API_ORCHESTRATION_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects update-drain API maintenance orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/updates/drain/lease.rs",
    contents: `
      use ctx_daemon::daemon::maintenance;
      async fn helper(execution: ExecutionHandle, state: CoreHandle, error: anyhow::Error) {
        let _ = execution.begin_update_drain("daemon_update".to_string(), "unknown".to_string()).await;
        let _ = execution.release_update_drain().await;
        let _ = execution.request_daemon_shutdown("desktop_quit".to_string()).await;
        let _ = state.local_shutdown_token();
        let _ = local_shutdown_token_authorized(&headers);
        let _ = logs::redact_sensitive(&error.to_string());
        let _ = begin_update_drain_error(BeginUpdateDrainError::Busy);
        let _ = daemon_shutdown_error(DaemonShutdownError::Reconcile(error));
        let _ = internal_error_response(error);
      }
    `,
    patterns: UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "update drain API imports daemon maintenance internals",
      "update drain API references low-level maintenance errors",
      "update drain API references low-level maintenance errors",
      "update drain API calls low-level daemon maintenance methods",
      "update drain API calls low-level daemon maintenance methods",
      "update drain API calls low-level daemon maintenance methods",
      "update drain API authorizes shutdown token locally",
      "update drain API authorizes shutdown token locally",
      "update drain API redacts maintenance errors locally",
      "update drain API owns maintenance default values",
      "update drain API owns maintenance default values",
      "update drain API owns maintenance error mapping helpers",
      "update drain API owns maintenance error mapping helpers",
      "update drain API owns maintenance error mapping helpers",
    ],
  );
});

test("daemon boundary guard scopes update-drain API orchestration roots", () => {
  for (const filePath of [
    "core/crates/ctx-http/src/api/updates/drain.rs",
    "core/crates/ctx-http/src/api/updates/drain/lease.rs",
    "core/crates/ctx-http/src/api/updates/drain/shutdown.rs",
  ]) {
    assert.deepEqual(
      updateDrainApiPatternsForPath(filePath),
      UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS,
    );
    assert.equal(
      apiPatternsForPath(filePath).includes(UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS[0]),
      true,
    );
  }
  assert.deepEqual(
    updateDrainApiPatternsForPath("core/crates/ctx-http/src/api/updates/check.rs"),
    [],
  );
});

test("daemon boundary guard rejects blob API storage orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/artifacts/blob.rs",
    contents: `
      use ctx_daemon::daemon::{BlobReadError, StoredImageBlob};
      use sha2::Digest;
      use tokio::fs as async_fs;
      async fn helper(state: CoreHandle) {
        let dir = state.data_root().join("blobs");
        tokio::fs::write(path, bytes).await?;
        tokio::fs::rename(tmp, path).await?;
        tokio::fs::read(path).await?;
        tokio::fs::metadata(path).await?;
        tokio::fs::remove_file(path).await?;
        tokio::fs::File::open(path).await?;
        let _ = tokio::fs::OpenOptions::new();
        fs::read(path).await?;
        fs::remove_file(path).await?;
        std::fs::read(path)?;
        let mut hasher = sha2::Sha256::new();
        let id = uuid::Uuid::new_v4();
        state.insert_blob(&id, sha, 1, "image/png", None, now).await?;
        state.get_blob(&id).await?;
      }
    `,
    patterns: BLOB_API_ORCHESTRATION_PATTERNS,
  });

  const names = violations.map((violation) => violation.name);
  for (const expected of [
    "blob API imports blob service contracts from daemon",
    "blob API accesses daemon data root directly",
    "blob API owns blob filesystem operations",
    "blob API owns blob checksum generation",
    "blob API owns blob id generation",
    "blob API accesses blob store metadata directly",
  ]) {
    assert(names.includes(expected), `expected violation: ${expected}`);
  }
  assert.equal(
    names.filter((name) => name === "blob API accesses blob store metadata directly").length,
    2,
  );
});

test("daemon boundary guard rejects multiline blob service contract daemon imports", () => {
  for (const [contents, expected] of [
    [
      `
        use ctx_daemon::daemon::{
          BlobReadError,
          ImageBlobStoreError,
        };
      `,
      "blob API imports blob service contracts from daemon",
    ],
    [
      `
        use ctx_daemon::{
          daemon::{
            StoredImageBlob,
          },
        };
      `,
      "blob API imports blob service contracts from nested daemon group",
    ],
  ]) {
    const violations = scanText({
      filePath: "core/crates/ctx-http/src/api/artifacts/blob/errors.rs",
      contents,
      patterns: BLOB_API_ORCHESTRATION_PATTERNS,
    });

    assert.deepEqual(
      violations.map((violation) => violation.name),
      [expected],
    );
  }
});

test("daemon boundary guard scopes blob API orchestration roots", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/artifacts/blob.rs").includes(
      BLOB_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/artifacts/blob/upload.rs").includes(
      BLOB_API_ORCHESTRATION_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/artifacts/session.rs").includes(
      BLOB_API_ORCHESTRATION_PATTERNS[0],
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

test("daemon boundary guard rejects execution API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/execution/linux_sandbox.rs",
    contents: `
      use ctx_linux_sandbox_runtime::{linux_sandbox_runtime_status, LinuxSandboxRuntimeStatus};
      use ctx_daemon::daemon::{
        maintenance as daemon_maintenance, WorkspacesHandle, StartExecutionLaunchRequest,
        LinuxSandboxRuntimeError,
      };
      use ctx_settings_model::{ExecutionMode, ExecutionSettings};
      async fn route(core: CoreHandle, workspaces: WorkspacesHandle, execution: ExecutionHandle) {
        let _ = core.data_root();
        let _ = workspaces.get_workspace(workspace_id).await;
        let _ = workspaces.effective_execution_settings_classified(workspace_id).await;
        let _ = execution.reject_new_execution_during_maintenance().await;
        let _ = execution.acquire_linux_sandbox_prepare_drain().await;
        let _ = resolve_workspace_launch_inputs(&workspaces, None).await;
        let _ = linux_sandbox_user_message("status");
        let _ = ctx_linux_sandbox_runtime::linux_sandbox_runtime_status(root).await;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/execution/linux_sandbox.rs"),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "execution API imports moved execution route contracts from daemon",
    "execution API imports Linux sandbox runtime directly",
    "execution API accesses daemon data root directly",
    "execution API owns workspace lookup",
    "execution API owns effective execution settings",
    "execution API owns maintenance drain",
    "execution API owns workspace launch input resolution",
    "execution API owns Linux sandbox user messages",
    "execution API calls Linux sandbox runtime by fully-qualified path",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }
});

test("daemon boundary guard rejects health and diagnostics API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/diagnostics.rs",
    contents: `
      use super::health::{build_health_response, HealthResp};
      use ctx_linux_sandbox_runtime::linux_sandbox_runtime_status;
      async fn route(core: CoreHandle, execution: ExecutionHandle, providers: ProvidersHandle) {
        let _ = ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION"));
        let _ = linux_sandbox_runtime_status(core.data_root()).await;
        let _ = logs::list_log_files(core.data_root()).await;
        let _ = logs::logs_dir(core.data_root());
        let _ = ctx_resource_utilization::process_limits::current_open_file_limit();
        let _ = core.storage_guard_snapshot();
        let _ = execution.startup_status().await;
        let _ = providers.provider_diagnostics_snapshot().await;
        let _response: DiagnosticsResp = todo!();
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/diagnostics.rs"),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "health/diagnostics API calls update service directly",
    "health/diagnostics API imports Linux sandbox runtime directly",
    "health/diagnostics API reads process limits directly",
    "health/diagnostics API reads observability logs directly",
    "health/diagnostics API accesses daemon data root directly",
    "health/diagnostics API reads storage guard directly",
    "health/diagnostics API reads execution startup status directly",
    "health/diagnostics API reads provider diagnostics directly",
    "health/diagnostics API owns health response assembly",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }
});

test("daemon boundary guard rejects resource utilization API local route contracts", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/resource_utilization.rs",
    contents: `
      use ctx_core::ids::WorkspaceId;
      use ctx_daemon::daemon::resource_utilization as daemon_resource_utilization;
      async fn route(state: WorkspacesHandle, raw: String) {
        let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&raw).unwrap());
        let _snapshot: ctx_resource_utilization::ResourceUtilizationSnapshot =
          state.workspace_resource_utilization_snapshot(workspace_id).await.unwrap();
        let _error: daemon_resource_utilization::ResourceUtilizationSnapshotError = todo!();
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/resource_utilization.rs"),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "resource utilization API parses workspace ids locally",
    "resource utilization API exposes raw resource snapshot",
    "resource utilization API imports low-level resource errors",
    "resource utilization API calls typed resource facade directly",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }
  assert(
    apiPatternsForPath("core/crates/ctx-http/src/api/resource_utilization.rs").includes(
      RESOURCE_UTILIZATION_API_ROUTE_CONTRACT_PATTERNS[0],
    ),
    "resource utilization route should be covered by scoped route-contract guard",
  );
});

test("daemon boundary guard rejects daemon health package-version fallback", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-daemon/src/daemon/health.rs",
    contents: `
      fn health_snapshot() {
        let _ = ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION"));
      }
    `,
    patterns: DAEMON_HEALTH_VERSION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["daemon health uses daemon crate package version directly"],
  );
});

test("daemon boundary guard rejects logs API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/logs_api.rs",
    contents: `
      use ctx_observability::logs;
      async fn route(core: CoreHandle) {
        let line = format!("{} [{}] {}", chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), "info", "message");
        logs::append_desktop_log_line(core.data_root(), &line).await?;
        logs::open_logs_folder(core.data_root()).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/logs_api.rs"),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "logs API imports observability logs directly",
    "logs API calls log filesystem helpers directly",
    "logs API assembles desktop log line locally",
    "logs API accesses daemon data root directly",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }
});

test("daemon boundary guard rejects update API orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/updates/appimage/download.rs",
    contents: `
      async fn route(core: CoreHandle) {
        let _status: ctx_update_service::ManagedDaemonAutoUpdateStatus = todo!();
        let _response: DownloadAppImageResp = todo!();
        let _ = ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION"));
        let _ = ctx_update_service::download_verified_appimage_candidate(req).await;
        let _ = logs::redact_sensitive("secret");
        let _ = core.data_root();
      }
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/updates/appimage/download.rs",
    ),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "update API calls update service directly",
    "update API redacts errors locally",
    "update API accesses daemon data root directly",
    "update API owns managed auto-update DTO",
    "update API owns update response DTO assembly",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }
});

test("daemon boundary guard allows update API route-contract imports", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/updates/check.rs",
    contents: `
      use ctx_update_service::route_contract::{
        UpdateCheckSnapshot,
        UpdateRouteError,
        UpdateRouteErrorKind,
      };
      async fn route(core: CoreHandle) -> Result<Json<UpdateCheckSnapshot>, UpdateRouteError> {
        todo!()
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/updates/check.rs"),
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard rejects workspace registration/config orchestration", () => {
  const registrationViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/registry/create.rs",
    contents: `
      use ctx_repo_onboarding_service::{prepare_workspace_registration, WorkspaceRegistrationError};
      async fn create(workspaces: WorkspacesHandle) {
        let _ = prepare_workspace_registration(root).await;
        workspaces.record_workspace_registered().await;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/registry/create.rs"),
  });
  assert.deepEqual(
    registrationViolations.map((violation) => violation.name),
    [
      "workspace registration API imports registration service directly",
      "workspace registration API owns registration preparation",
      "workspace registration API owns registration error type",
      "workspace registration API owns registration telemetry sequencing",
    ],
  );

  const primaryBranchViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/config_ops/primary_branch.rs",
    contents: `
      async fn update() {
        ctx_repo_onboarding_service::validate_workspace_primary_branch(root, branch).await?;
      }
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/config_ops/primary_branch.rs",
    ),
  });
  assert.deepEqual(
    primaryBranchViolations.map((violation) => violation.name),
    [
      "workspace registration API imports registration service directly",
      "workspace primary branch API owns branch validation",
    ],
  );
});

test("daemon boundary guard rejects workspace execution-config orchestration", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/config_ops/execution.rs",
    contents: `
      use ctx_settings_service::{
        apply_workspace_execution_settings_override,
        validate_workspace_execution_settings_override,
      };
      async fn route(workspaces: WorkspacesHandle) {
        let settings = workspaces.load_settings().await?;
        let override_config = workspaces.load_workspace_execution_override(workspace_id).await?;
        apply_workspace_execution_settings_override(&mut effective, &override_config)?;
        validate_workspace_execution_settings_override(&effective, &requested)?;
        let _ = workspaces.shared_vm_container_runtime_available();
        let requested = build_workspace_execution_config_override(environment, network, allowlist);
        let response = project_workspace_execution_config(source, &effective);
        workspaces.update_workspace_execution_config(workspace_id, update).await?;
      }
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/config_ops/execution.rs",
    ),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "workspace execution config API imports settings service directly",
    "workspace execution config API applies overrides directly",
    "workspace execution config API validates overrides directly",
    "workspace execution config API loads daemon settings directly",
    "workspace execution config API loads workspace execution override directly",
    "workspace execution config API checks sandbox runtime directly",
    "workspace execution config API builds execution override directly",
    "workspace execution config API projects execution config directly",
    "workspace execution config API persists execution config directly",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }
});

test("daemon boundary guard rejects workspace config route context backdoors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management.rs",
    contents: `
      pub(in crate::api) async fn get_execution_config(
          State(workspaces): State<WorkspacesHandle>,
          Path(id): Path<String>,
      ) -> Result<Json<WorkspaceExecutionConfigSnapshot>, (StatusCode, Json<ApiErrorResp>)> {
          let ctx = require_workspace_ctx(&workspaces, &id).await?;
          let workspace = require_workspace(&workspaces, ctx.workspace_id).await?;
          let again = workspaces.get_workspace(ctx.workspace_id).await?;
          todo!()
      }

      pub(in crate::api) async fn update_merge_queue_config(
          State(workspaces): State<WorkspacesHandle>,
          Path(id): Path<String>,
      ) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
          let ctx = require_workspace_ctx(&workspaces, &id).await?;
          todo!()
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management.rs"),
  });

  const names = violations.map((violation) => violation.name);
  for (const expected of [
    "workspace config API requires workspace context in HTTP",
    "workspace config API loads workspace in HTTP",
    "workspace config API fetches workspace directly in HTTP",
  ]) {
    assert(names.includes(expected), `expected ${expected}; saw ${names.join(", ")}`);
  }
});

test("daemon boundary guard scopes workspace registration/config API roots", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/registry/create.rs").includes(
      WORKSPACE_REGISTRATION_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/config_ops/execution.rs").includes(
      WORKSPACE_EXECUTION_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management.rs").includes(
      WORKSPACE_REGISTRATION_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management.rs").includes(
      WORKSPACE_EXECUTION_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management.rs").includes(
      WORKSPACE_CONFIG_ROUTE_CONTEXT_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/config_ops/merge_queue.rs").includes(
      WORKSPACE_EXECUTION_CONFIG_API_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects workspace management config backdoors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
    contents: `
      use ctx_core::models::Workspace;
      use ctx_workspace_config as workspace_config;

      pub(super) struct WorkspaceRequestContext;
      pub(super) struct UpdateWorkspaceConfigResp;
      pub(super) struct UpdateMergeQueueConfigReq;
      pub(super) struct WorkspaceMergeQueueConfigResp;
      pub(super) struct UpdateWorktreeBootstrapReq;
      pub(super) struct WorkspaceWorktreeBootstrapConfigResp;

      async fn update(workspaces: WorkspacesHandle, id: String) {
        let ctx = require_workspace_ctx(&workspaces, &id).await?;
        let _workspace = require_workspace(&workspaces, ctx.workspace_id).await?;
        workspaces.update_workspace_merge_queue_config(
          ctx.workspace_id,
          workspace_config::MergeQueueConfigUpdate {
            enabled: true,
            target_branch: None,
            verify_commands: Vec::new(),
            push_on_success: None,
            push_remote: None,
            push_branch: None,
            canonical_sync: None,
          },
        ).await?;
        workspaces.load_worktree_bootstrap_config(ctx.workspace_id).await?;
        workspaces.update_worktree_bootstrap_config(
          ctx.workspace_id,
          workspace_config::WorktreeBootstrapConfigUpdate::default(),
        ).await?;
      }
    `,
    patterns: apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs"),
  });

  const names = new Set(violations.map((violation) => violation.name));
  for (const expected of [
    "workspace management config API owns workspace context lookup",
    "workspace management config API exposes raw Workspace",
    "workspace management config API owns local route DTOs",
    "workspace management config API builds raw workspace config updates",
    "workspace management config API calls raw config facade methods",
    "workspace management config API imports workspace config directly",
  ]) {
    assert(names.has(expected), `expected ${expected}; saw ${[...names].join(", ")}`);
  }

  const promptAndModelViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
    contents: `
      use ctx_core::ids::WorkspaceId;
      use ctx_observability::logs;
      use ctx_workspace_config::{
        AgentSystemPromptAppendConfig,
        AgentSystemPromptAppendSource,
        SubagentSystemPromptAppendConfig,
      };
      struct UpdateWorkspaceProviderModelPreferenceReq;
      struct WorkspaceProviderModelPreferenceResp;
      struct UpdateAgentSystemPromptConfigReq;
      struct AgentSystemPromptConfigResponse;
      struct UpdateSubagentSystemPromptConfigReq;
      struct SubagentSystemPromptConfigResponse;
      async fn handler(workspaces: WorkspacesHandle, error: WorkspaceProviderModelPreferenceError) {
        let workspace_id = uuid::Uuid::parse_str(id).map(WorkspaceId)?;
        let raw = WorkspaceProviderModelPreference {
          provider_id: provider_id.to_string(),
          preferred_model_id: None,
        };
        let _ = workspaces.get_workspace_provider_model_preference(workspace_id, provider_id).await?;
        let _ = WorkspacesHandle::set_workspace_provider_model_preference(
          &workspaces,
          workspace_id,
          provider_id,
          None,
        ).await?;
        let _ = workspaces.load_agent_system_prompt_append(workspace_id).await?;
        let _ = WorkspacesHandle::update_subagent_system_prompt_append(
          &workspaces,
          workspace_id,
          None,
        ).await?;
        let _ = source_label(AgentSystemPromptAppendSource::Config);
        let _ = configured_append(&None);
        let _ = logs::redact_sensitive(error.to_string());
      }
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
    ),
  });

  const promptAndModelNames = new Set(
    promptAndModelViolations.map((violation) => violation.name),
  );
  for (const expected of [
    "workspace management config API parses route ids directly",
    "workspace management config API exposes raw provider preference contracts",
    "workspace management config API exposes raw prompt config contracts",
    "workspace management config API owns local route DTOs",
    "workspace management config API calls raw prompt and model config facades",
    "workspace management config API owns prompt response projection",
    "workspace management config API redacts prompt/model errors locally",
    "workspace management config API imports workspace config directly",
  ]) {
    assert(
      promptAndModelNames.has(expected),
      `expected ${expected}; saw ${[...promptAndModelNames].join(", ")}`,
    );
  }

  const routeViolations = scanText({
    filePath: "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
    contents: `
      async fn get_workspace_provider_model_preference(
          State(workspaces): State<WorkspacesHandle>,
          Path((id, provider_id)): Path<(String, String)>,
      ) -> Result<Json<WorkspaceProviderModelPreferenceRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
          workspaces
              .workspace_provider_model_preference_for_route(
                  WorkspaceProviderModelPreferenceRouteParams::new(id, provider_id),
              )
              .await
              .map(Json)
      }
    `,
    patterns: apiPatternsForPath(
      "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
    ),
  });
  assert.deepEqual(routeViolations, []);
});

test("daemon boundary guard rejects moved workspace config contracts from daemon", () => {
  for (const [filePath, contents, expectedName] of [
    [
      "core/crates/ctx-http/src/api/workspaces.rs",
      `
        use ctx_daemon::daemon::{
          UpdateWorkspaceExecutionConfigRequest,
          WorkspaceMergeQueueConfigRouteResponse,
          WorkspacesHandle,
        };
      `,
      "workspace management config API imports moved config contracts from daemon",
    ],
    [
      "core/crates/ctx-http/src/api/workspaces/management.rs",
      `
        use ctx_daemon::daemon::workspaces::{
          UpdateWorkspaceMergeQueueConfigRequest,
          WorkspaceExecutionConfigSnapshot,
        };
      `,
      "workspace management config API imports moved config contracts from daemon",
    ],
    [
      "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
      `
        use ctx_daemon::daemon::{
          workspaces::{WorkspaceProviderModelPreferenceRouteParams, WorkspaceProviderModelPreferenceRouteResponse},
          WorkspacesHandle,
        };
      `,
      "workspace management config API imports moved config contracts from nested daemon workspaces group",
    ],
    [
      "core/crates/ctx-http/src/api/workspaces/management/prompt_config/agent.rs",
      `use ctx_daemon::daemon::workspaces as daemon_workspaces;`,
      "workspace management config API imports daemon workspaces root",
    ],
    [
      "core/crates/ctx-http/src/api/workspaces/management/prompt_config/subagent.rs",
      `use ctx_daemon::daemon::{workspaces, WorkspacesHandle};`,
      "workspace management config API imports daemon workspaces root",
    ],
    [
      "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
      `use ctx_daemon::daemon::workspaces::*;`,
      "workspace management config API imports daemon workspaces root",
    ],
    [
      "core/crates/ctx-http/src/api/workspaces/attachments.rs",
      `
        use ctx_daemon::daemon::{
          UpdateWorkspaceExecutionConfigRequest,
          WorkspacesHandle,
        };
      `,
      "workspace management config API imports moved config contracts from daemon",
    ],
  ]) {
    const violations = scanText({
      filePath,
      contents,
      patterns: apiPatternsForPath(filePath),
    });
    assert(
      violations.some((violation) => violation.name === expectedName),
      `expected ${expectedName} for ${filePath}; saw ${violations
        .map((violation) => violation.name)
        .join(", ")}`,
    );
  }
});

test("daemon boundary guard scopes workspace management config roots", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/context.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/config_ops/merge_queue.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/prompt_config.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/prompt_config/agent.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/workspaces/attachments.rs").includes(
      WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS[0],
    ),
    true,
  );
});

test("daemon boundary guard rejects daemon updates package-version fallback", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-daemon/src/daemon/updates.rs",
    contents: `
      fn check_updates() {
        let _ = ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION"));
      }
    `,
    patterns: DAEMON_UPDATES_VERSION_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["daemon updates uses daemon crate package version directly"],
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

test("daemon boundary guard rejects task/session publication effect state backdoors", () => {
  const hostViolations = scanTaskSessionPublicationEffectsStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/task_session_effects.rs",
    contents: `
      use crate::daemon::DaemonState;
      struct TaskPublicationHost {
        state: Arc<DaemonState>,
      }
      impl TaskPublicationHost {
        fn daemon_state(&self) -> &DaemonState { todo!() }
      }
    `,
  });
  assert.deepEqual(
    hostViolations.map((violation) => violation.name),
    [
      "task/session publication effects depend on broad daemon state",
      "task/session publication effects depend on broad daemon state",
      "task/session publication effects depend on broad daemon state",
      "task/session publication effects expose generic state escape hatch",
      "task/session publication effects expose generic state escape hatch",
    ],
  );

  const methodOnlyHostViolations = scanTaskSessionPublicationEffectsStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/task_session_effects.rs",
    contents: `
      impl TaskPublicationHost {
        pub(crate) fn daemon_state(&self) -> NarrowDependency { todo!() }
      }
    `,
  });
  assert.deepEqual(
    methodOnlyHostViolations.map((violation) => violation.name),
    ["task/session publication effects expose generic state escape hatch"],
  );

  const publicationViolations = scanTaskSessionPublicationEffectsStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/sessions/runtime/publication.rs",
    contents: `
      mod task_delta;
      struct HttpTaskDeltaRefreshHost {
        daemon_state: Arc<DaemonState>,
      }
    `,
  });
  assert.deepEqual(
    publicationViolations.map((violation) => violation.name),
    [
      "session runtime publication uses stale weak daemon-state task refresh host",
      "session runtime publication uses stale weak daemon-state task refresh host",
      "session runtime publication uses stale weak daemon-state task refresh host",
    ],
  );

  const handleViolations = scanTaskSessionPublicationEffectsStateRatchet({
    filePath: "core/crates/ctx-daemon/src/daemon/handle.rs",
    contents: `
      impl DaemonHandle {
        fn task_session_admission_effects(&self) -> Arc<TaskAdmissionSessionEffects> {
          let state = Arc::clone(&self.state);
          let publish_event = Arc::new(move |event| {
            let state = Arc::clone(&state);
            Box::pin(async move { state.publish_event(event).await })
          });
          todo!()
        }
      }
    `,
  });
  assert.deepEqual(
    handleViolations.map((violation) => violation.name),
    [
      "task/session publication assembly hides full daemon state closure",
      "task/session publication assembly hides full daemon state closure",
    ],
  );
});

test("checked-in ctx-http API satisfies the daemon boundary", () => {
  assert.deepEqual(scanRepo(), []);
});
