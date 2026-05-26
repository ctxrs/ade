use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment, SessionTurn, SessionTurnStatus};
#[cfg(test)]
use ctx_store::{StoreManager, StoreManagerConfig};

use crate::daemon::scheduler::reconcile_turn_terminal_state;
use ctx_provider_runtime::provider_usage;

mod activity;
pub mod blobs;
pub mod diagnostics;
pub mod dictation;
pub mod execution_effective;
pub mod execution_setup;
pub mod git_status;
mod handle;
pub mod health;
mod lifecycle;
mod listener;
pub mod logs;
pub mod maintenance;
mod managed_auto_update;
mod managed_installs_host;
mod mcp_auth;
mod memleak_debug;
pub mod merge_queue;
pub mod mobile_access;
mod mobile_startup;
pub mod org_policy;
mod org_policy_route;
mod provider_capability_hosts;
mod provider_child_reclassifier;
pub mod provider_guard;
mod provider_launch_host;
pub mod provider_restart;
mod provider_route_handles;
mod provider_runtime;
pub mod providers;
pub mod repo_onboarding;
pub mod resource_governance;
pub mod resource_telemetry;
pub mod resource_utilization;
mod retention;
mod route_capabilities;
mod route_files;
mod route_handles;
mod runtime;
pub mod scheduler;
mod session_control_effects;
mod session_route_handles;
pub mod sessions;
pub mod settings;
mod state;
pub mod storage_guard;
mod task_route_handles;
mod task_session_effects;
pub mod tasks;
pub mod terminals;
#[cfg(any(test, feature = "test-support"))]
mod test_support_access;
pub mod tool_cgroup;
pub mod updates;
pub mod web_sessions;
mod workspace_route_handles;
#[cfg(test)]
mod workspace_runtime;
pub mod workspaces;

#[cfg(test)]
pub(in crate::daemon) use self::runtime::spawn_startup_provider_status_refresh;
use activity::reconcile_running_turns;
pub use activity::reconcile_running_turns_with_reason;
pub use activity::{
    daemon_sandbox_work_activity_summary, daemon_turn_activity_summary, ActiveTurnRecord,
    DaemonSandboxWorkActivitySummary, DaemonTurnActivitySummary,
};
pub use blobs::{BlobHandle, OpenedBlob};
pub use diagnostics::DiagnosticsSnapshotError;
pub(in crate::daemon) use git_status::{WorktreeVcsExecutionHost, WorktreeVcsRuntimeHost};
pub(crate) use handle::route_handles_from_state;
pub(in crate::daemon) use handle::ProviderWorkspaceLaunchRuntime;
pub use handle::{
    DaemonHandle, DaemonShutdownHandle, ExecutionLaunchHandle, LinuxSandboxRuntimeHandle,
    MergeQueueApiHandle, MobileRuntimeHandle, MobileSecureProxyHandle, ResourceUtilizationHandle,
    RunArchiveHandle, SettingsHandle, TerminalRouteHandle, UpdateActivityHandle, UpdateDrainHandle,
    WebSessionRouteHandle, WorkspaceActiveHandle, WorkspaceStreamHandle, WorkspaceVcsStreamHandle,
};
pub use health::HealthSnapshotError;
pub(in crate::daemon) use lifecycle::spawn_deferred_daemon_shutdown;
pub(in crate::daemon) use lifecycle::{DaemonShutdownHost, DaemonShutdownHostParts};
#[cfg(test)]
pub use listener::daemon_public_base_url_from_env;
pub use mcp_auth::issue_provider_session_mcp_token;
pub use mcp_auth::{
    emit_mcp_token_denied, issue_provider_session_mcp_token_with_capabilities,
    require_scoped_mcp_session_context, revoke_provider_session_mcp_token, verify_mcp_auth_token,
    ScopedMcpSessionAccessError,
};
pub use provider_route_handles::{
    ProviderAccountsHandle, ProviderAdminHandle, ProviderAuthImportHandle, ProviderBootstrapHandle,
    ProviderHarnessConfigHandle, ProviderInstallHandle, ProviderOptionsHandle,
    ProviderStatusHandle, ProviderUsageHandle, ProviderWorkspaceAuthHandle,
};
#[cfg(test)]
pub use retention::prune_archived_session_data_for_all_workspaces;
pub use route_capabilities::{DaemonRouteHandles, DaemonShutdownSignal};
pub use route_files::RouteFileDownloadError;
pub use route_handles::{
    AuthHandle, DiagnosticsHandle, DictationHandle, HealthHandle, LogsHandle, MobileStoreHandle,
    OrgPolicyHandle, RepoOnboardingHandle, RequestBaseHandle, TelemetryHandle, UpdateReleaseHandle,
};
pub use runtime::{bootstrap_daemon_runtime, DaemonRuntime};
pub use session_control_effects::SessionControlHandle;
pub use session_route_handles::{
    SessionArtifactsHandle, SessionFileCompletionsHandle, SessionMessageCommandHandle,
    SessionReadModelsHandle, SessionSubagentMcpReadHandle, SessionSubagentReadHandle,
    SessionTitleModelModeHandle, SessionVcsHandle,
};
pub use sessions::subagents::SessionSubagentMcpControlHandle;
pub use sessions::title_generation::TitleGenerationLocalHandle;
pub use sessions::DemoSeedTranscriptHandle;
pub(in crate::daemon) use state::{
    session_store_access_anyhow, ProtectedWorkspaceStoreLookup, SessionStoreLookup,
    WeakSessionStoreLookup,
};
pub use state::{AppRuntimeFlags, DaemonState};
pub use state::{
    CacheSweepConfig, SessionStoreAccessError, StoreLookup, TimedEntry, WorkspaceStoreAccessError,
};
pub use task_route_handles::{
    TaskCreationHandle, TaskLifecycleHandle, TaskListingHandle, TaskReadStateHandle,
    TaskSessionAdmissionHandle, TaskSessionListingHandle, TaskTitleHandle,
};
pub use workspace_route_handles::{
    WorkspaceAttachmentsHandle, WorkspaceDeletionHandle, WorkspaceExecutionConfigHandle,
    WorkspaceFileCompletionsHandle, WorkspaceHarnessContainerHandle,
    WorkspaceMergeQueueConfigHandle, WorkspaceOrgPolicyHandle, WorkspacePrimaryBranchHandle,
    WorkspacePromptBootstrapConfigHandle, WorkspaceProviderModelPreferenceHandle,
    WorkspaceRegistryHandle, WorkspaceWorktreeHandle,
};
pub use workspaces::{WorkspaceStreamAccessError, WorkspaceStreamRouteAdmission};

#[cfg(test)]
mod tests;
