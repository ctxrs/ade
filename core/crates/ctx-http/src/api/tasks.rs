use std::collections::HashSet;
use std::path::Path as StdPath;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[path = "tasks/creation.rs"]
mod creation;
mod handlers;
#[path = "tasks/task_deletion.rs"]
mod task_deletion;
#[path = "tasks/task_title.rs"]
mod task_title;
use crate::daemon::workspaces::{
    execution_environment_from_settings, BranchCleanupErrorMode, TaskWorktreeCleanupTarget,
};
pub(in crate::api) use creation::*;
pub(in crate::api) use handlers::*;
pub(super) use task_deletion::{delete_loaded_task_with_cleanup, delete_task};
pub(super) use task_title::update_task_title;

use super::errors::ApiErrorResp;
#[cfg(test)]
use crate::daemon::DaemonHandle;
#[cfg(test)]
use crate::daemon::DaemonState;
use crate::daemon::{
    ProvidersHandle, SessionsHandle, TasksHandle, TransportHandle, WorkspacesHandle,
};
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
#[cfg(test)]
use ctx_core::models::SandboxBinding;
#[cfg(test)]
use ctx_core::models::Worktree;
use ctx_core::models::{
    ExecutionEnvironment, Session, Task, Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor,
};
use ctx_observability::logs;
use ctx_store::Store;

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

#[derive(Clone)]
pub(super) struct TaskApiHandles {
    pub(super) tasks: TasksHandle,
    pub(super) sessions: SessionsHandle,
    pub(super) providers: ProvidersHandle,
    pub(super) workspaces: WorkspacesHandle,
}

impl TaskApiHandles {
    pub(super) fn new(
        tasks: TasksHandle,
        sessions: SessionsHandle,
        providers: ProvidersHandle,
        workspaces: WorkspacesHandle,
    ) -> Self {
        Self {
            tasks,
            sessions,
            providers,
            workspaces,
        }
    }
}

#[cfg(test)]
pub(super) fn task_api_task_state(state: &Arc<DaemonState>) -> State<TasksHandle> {
    State(DaemonHandle::new(Arc::clone(state)).tasks())
}

fn task_request_matches(existing: &Task, title: &str, description: &Option<String>) -> bool {
    existing.title == title && existing.description.as_deref() == description.as_deref()
}

fn task_lifecycle_status(error: crate::daemon::tasks::TaskLifecycleError) -> StatusCode {
    match error {
        crate::daemon::tasks::TaskLifecycleError::NotFound => StatusCode::NOT_FOUND,
        crate::daemon::tasks::TaskLifecycleError::Internal(error) => {
            tracing::warn!("task lifecycle operation failed: {error:#}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn task_session_create_status(error: crate::daemon::tasks::TaskSessionCreateError) -> StatusCode {
    match error {
        crate::daemon::tasks::TaskSessionCreateError::BadRequest => StatusCode::BAD_REQUEST,
        crate::daemon::tasks::TaskSessionCreateError::NotFound => StatusCode::NOT_FOUND,
        crate::daemon::tasks::TaskSessionCreateError::Conflict => StatusCode::CONFLICT,
        crate::daemon::tasks::TaskSessionCreateError::Internal(error) => {
            tracing::warn!("task session creation failed: {error:#}");
            crate::api::shared::status_code_for_internal_error(&error)
        }
    }
}
#[cfg(test)]
mod cleanup_lifecycle_tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod storage_admission_http_tests;

#[cfg(test)]
mod tests;
