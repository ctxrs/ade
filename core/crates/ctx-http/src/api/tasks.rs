use std::collections::HashSet;
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[path = "tasks/creation.rs"]
mod creation;
mod execution;
mod handlers;
#[path = "tasks/task_deletion.rs"]
mod task_deletion;
#[path = "tasks/task_title.rs"]
mod task_title;
#[path = "tasks/worktree_lifecycle.rs"]
mod worktree_lifecycle;
pub(in crate::api) use creation::*;
pub(crate) use execution::resolve_existing_worktree_execution;
pub(crate) use execution::sandbox_execution_settings_from_binding;
#[allow(unused_imports)]
pub(in crate::api) use execution::*;
pub(in crate::api) use handlers::*;
pub(super) use task_deletion::{delete_loaded_task_with_cleanup, delete_task};
pub(super) use task_title::update_task_title;
#[cfg(test)]
pub(crate) use worktree_lifecycle::branch_exists;
pub(crate) use worktree_lifecycle::{
    cleanup_task_worktrees, ensure_worktree_attached, execution_environment_from_settings,
    managed_worktree_root, persist_provisioned_worktree, provision_worktree_for_execution,
    rematerialize_sandbox_binding_for_worktree, retry_global_index_write, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};

use super::errors::ApiErrorResp;
use super::sessions::schedule_session_title_generation;
use super::shared::session_root_kind_for_worktree;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::scheduler::SchedulerCommand;
use crate::vcs_hooks;
use crate::worktree_bootstrap;
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Message, MessageDelivery, MessageRole, SandboxBinding, SandboxProfile,
    Session, SessionEventType, SessionTurn, SessionTurnStatus, Task, TaskDeltaKind, VcsKind,
    Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor, Worktree,
};
use ctx_fs::git::delete_branch;
use ctx_fs::vcs;
use ctx_fs::worktrees::{create_worktree, managed_worktree_path};
use ctx_observability::logs;
use ctx_observability::ops_events::OpsEvent;
use ctx_observability::telemetry::TelemetryEvent;
use ctx_settings_model::{ExecutionMode, ExecutionSettings};
use ctx_store::{is_unique_constraint_violation, Store};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateTaskReq {
    #[serde(default)]
    id: Option<String>,
    title: String,
    description: Option<String>,
    #[serde(default)]
    default_session: Option<CreateTaskDefaultSessionReq>,
}

fn task_request_matches(existing: &Task, title: &str, description: &Option<String>) -> bool {
    existing.title == title && existing.description.as_deref() == description.as_deref()
}
mod snapshot_state;
pub(crate) use snapshot_state::load_workspace_active_snapshot_state;

#[cfg(test)]
mod cleanup_lifecycle_tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod storage_admission_http_tests;

#[cfg(test)]
mod tests;
