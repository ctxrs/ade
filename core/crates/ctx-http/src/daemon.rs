use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use directories::BaseDirs;
use serde::Serialize;
use serde_json::json;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment, SessionTurn, SessionTurnStatus};
use ctx_store::{Store, StoreManager, StoreManagerConfig};

use crate::api;
use crate::daemon::scheduler::reconcile_turn_terminal_state;
use ctx_observability::telemetry::TelemetryConfig;
use ctx_provider_runtime::provider_usage;

mod activity;
mod auth;
pub(crate) mod execution_effective;
pub mod git_status;
pub(crate) mod installer;
mod lifecycle;
mod listener;
mod managed_auto_update;
mod mcp_auth;
mod memleak_debug;
pub(crate) mod merge_queue;
mod mobile_startup;
mod provider_child_reclassifier;
pub mod provider_guard;
pub(crate) mod provider_launch;
pub mod provider_restart;
mod provider_runtime;
pub(crate) mod resource_governance;
pub mod resource_telemetry;
mod retention;
pub mod scheduler;
mod serve;
pub(crate) mod sessions;
mod state;
pub(crate) mod storage_guard;
pub(crate) mod tool_cgroup;
mod workspace_init;
#[cfg(test)]
mod workspace_runtime;
pub(crate) mod workspaces;

pub use self::serve::serve;
#[cfg(test)]
pub(in crate::daemon) use self::serve::spawn_startup_provider_status_refresh;
use activity::reconcile_running_turns;
pub(crate) use activity::reconcile_running_turns_with_reason;
pub use activity::{
    daemon_sandbox_work_activity_summary, daemon_turn_activity_summary, ActiveTurnRecord,
    DaemonSandboxWorkActivitySummary, DaemonTurnActivitySummary,
};
pub use ctx_provider_runtime::{CachedProviderOptions, CachedProviderVerify};
pub use ctx_update_service::UpdateDrainState;
pub use ctx_workspace_services::file_completions::CachedFileCompletions;
pub(crate) use lifecycle::spawn_deferred_daemon_shutdown;
#[cfg(test)]
pub(crate) use lifecycle::{collect_provider_adapters_for_shutdown, shutdown_provider_adapters};
#[cfg(test)]
pub(crate) use listener::daemon_public_base_url_from_env;
pub use mcp_auth::issue_provider_session_mcp_token;
pub(crate) use mcp_auth::{
    emit_mcp_token_denied, issue_provider_session_mcp_token_with_capabilities,
    revoke_provider_session_mcp_token, verify_mcp_auth_token, McpAuthCapabilities, McpAuthContext,
};
#[cfg(test)]
pub(crate) use retention::prune_archived_session_data_for_all_workspaces;
pub(crate) use state::AttachmentMaterializationTask;
pub(crate) use state::WorktreeVcsDirtyBits;
pub use state::{
    AppRuntimeFlags, AppState, CacheSweepConfig, CacheSweepStats, GitStatusSnapshotCacheEntry,
    SessionHeadCacheKey, StoreLookup, TimedEntry, WorkspaceActiveHeadCacheEntry,
    WorkspaceActiveSnapshotCacheEntry, WorktreeVcsSnapshotCacheEntry,
};
pub use workspace_init::init_workspace;

#[cfg(test)]
mod tests;
