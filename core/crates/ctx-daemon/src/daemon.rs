use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;

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
mod provider_child_reclassifier;
pub mod provider_guard;
mod provider_launch_host;
pub mod provider_restart;
mod provider_runtime;
pub mod providers;
pub mod repo_onboarding;
pub mod resource_governance;
pub mod resource_telemetry;
pub mod resource_utilization;
mod retention;
mod route_files;
mod runtime;
pub mod scheduler;
pub mod sessions;
pub mod settings;
mod state;
pub mod storage_guard;
pub mod tasks;
pub mod telemetry_export;
pub mod terminals;
#[cfg(any(test, feature = "test-support"))]
mod test_support_access;
pub mod tool_cgroup;
pub mod updates;
pub mod web_sessions;
mod workspace_init;
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
pub use blobs::{BlobReadError, ImageBlobStoreError, OpenedBlob, StoredImageBlob};
pub use diagnostics::{DaemonDiagnosticsSnapshot, DiagnosticsSnapshotError};
pub use dictation::DictationConfigError;
pub use execution_setup::{
    LinuxSandboxActivationMode, LinuxSandboxRuntimeError, LinuxSandboxRuntimeOperation,
    LinuxSandboxRuntimePrepareResult, LinuxSandboxRuntimeStatus, StartExecutionLaunchError,
    StartExecutionLaunchRequest,
};
pub use handle::{
    CoreHandle, DaemonHandle, ExecutionHandle, ProvidersHandle, SessionsHandle, TasksHandle,
    TelemetryHandle, TransportHandle, WorkspaceStreamHandle, WorkspacesHandle,
};
pub use health::{DaemonHealthSnapshot, HealthCompatibility, HealthSnapshotError};
pub use lifecycle::spawn_deferred_daemon_shutdown;
#[cfg(test)]
pub use listener::daemon_public_base_url_from_env;
pub use maintenance::{
    BeginUpdateDrainRouteRequest, BeginUpdateDrainRouteResult, MaintenanceRouteError,
    MaintenanceRouteErrorKind, ReleaseUpdateDrainRouteRequest, ReleaseUpdateDrainRouteResult,
    ShutdownDaemonRouteRequest, ShutdownDaemonRouteResult,
};
pub use mcp_auth::issue_provider_session_mcp_token;
pub use mcp_auth::{
    emit_mcp_token_denied, issue_provider_session_mcp_token_with_capabilities,
    require_scoped_mcp_session_context, revoke_provider_session_mcp_token, verify_mcp_auth_token,
    ScopedMcpSessionAccessError,
};
#[cfg(test)]
pub use retention::prune_archived_session_data_for_all_workspaces;
pub use route_files::{RouteFileDownloadError, TextRouteDownload};
pub use runtime::{bootstrap_daemon_runtime, DaemonRuntime};
pub use sessions::{
    AuthenticateSessionRouteRequest, SessionControlRouteError, SessionControlRouteErrorKind,
    SessionEventsRouteQuery, SessionEventsRouteResponse, SessionFileCompletionsRouteQuery,
    SessionFileCompletionsRouteResponse, SessionHeadRouteQuery, SessionHeadRouteResponse,
    SessionHistoryRouteQuery, SessionHistoryRouteResponse, SessionReadModelRouteError,
    SessionReadModelRouteErrorKind, SessionRouteParams, SessionSnapshotRouteQuery,
    SessionSnapshotRouteResponse, SessionStateRouteResponse, SessionTurnToolsRouteParams,
    SessionTurnToolsRouteResponse, SubmitAskUserQuestionRouteRequest,
    SubmitAskUserQuestionRouteResponse,
};
pub use settings::{SettingsRouteError, SettingsRouteErrorKind};
pub use state::{AppRuntimeFlags, DaemonState};
pub use state::{
    AttachmentMaterializationTask, CacheSweepConfig, SessionStoreAccessError, StoreLookup,
    TimedEntry, WorkspaceStoreAccessError,
};
pub use telemetry_export::{TelemetryExportError, TelemetryExportErrorKind};
pub use updates::{
    ApplyAppImageUpdateRequest, ApplyAppImageUpdateResult, DownloadAppImageUpdateRequest,
    DownloadAppImageUpdateResult, UpdateActivitySnapshot, UpdateCheckSnapshot, UpdateRouteError,
    UpdateRouteErrorKind,
};
pub use workspace_init::init_workspace;
pub use workspaces::{
    CreateWorkspaceAttachmentRouteRequest, CreateWorkspaceRequest,
    DeleteWorkspaceAttachmentRouteRequest, SyncWorkspaceAttachmentsRouteRequest,
    UpdateWorkspaceExecutionConfigRequest, UpdateWorkspaceMergeQueueConfigRequest,
    UpdateWorkspacePrimaryBranchRequest, UpdateWorktreeBootstrapConfigRequest,
    WorkspaceActiveHeadBatchRouteResponse, WorkspaceActiveSnapshotRouteResponse,
    WorkspaceAttachmentRouteResponse, WorkspaceConfigUpdateResult,
    WorkspaceExecutionConfigSnapshot, WorkspaceHarnessContainerStatusRouteResponse,
    WorkspaceMergeQueueConfigRouteResponse, WorkspacePrimaryBranchSnapshot, WorkspaceRouteError,
    WorkspaceRouteErrorKind, WorkspaceRouteResponse, WorkspaceStreamAccessError,
    WorkspaceWorktreeBootstrapConfigRouteResponse, WorktreeRouteResponse,
};

#[cfg(test)]
mod tests;
