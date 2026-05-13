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
pub(crate) mod execution_effective;
pub(crate) mod execution_setup;
pub mod git_status;
mod lifecycle;
mod listener;
pub(crate) mod maintenance;
mod managed_auto_update;
mod managed_installs_host;
mod mcp_auth;
mod memleak_debug;
pub(crate) mod merge_queue;
pub(crate) mod mobile_access;
mod mobile_startup;
mod provider_child_reclassifier;
pub mod provider_guard;
mod provider_launch_host;
pub mod provider_restart;
mod provider_runtime;
pub(crate) mod providers;
pub(crate) mod resource_governance;
pub mod resource_telemetry;
pub(crate) mod resource_utilization;
mod retention;
pub mod scheduler;
mod serve;
pub(crate) mod sessions;
pub(crate) mod settings;
mod state;
pub(crate) mod storage_guard;
pub(crate) mod terminals;
pub(crate) mod tool_cgroup;
pub(crate) mod web_sessions;
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
pub(crate) use lifecycle::spawn_deferred_daemon_shutdown;
#[cfg(test)]
pub(crate) use listener::daemon_public_base_url_from_env;
pub use mcp_auth::issue_provider_session_mcp_token;
pub(crate) use mcp_auth::{
    emit_mcp_token_denied, issue_provider_session_mcp_token_with_capabilities,
    require_scoped_mcp_session_context, revoke_provider_session_mcp_token, verify_mcp_auth_token,
    ScopedMcpSessionAccessError,
};
#[cfg(test)]
pub(crate) use retention::prune_archived_session_data_for_all_workspaces;
pub use state::{AppRuntimeFlags, AppState};
pub(crate) use state::{
    AttachmentMaterializationTask, CacheSweepConfig, SessionStoreAccessError, StoreLookup,
    TimedEntry, WorkspaceStoreAccessError,
};
pub use workspace_init::init_workspace;

#[cfg(test)]
mod tests;
