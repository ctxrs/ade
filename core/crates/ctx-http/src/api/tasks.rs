use std::collections::HashSet;
use std::path::{Path as StdPath, PathBuf};
#[cfg(test)]
use std::sync::Arc;
use std::time::Instant;

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
    execution_environment_from_settings, retry_global_index_write, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};
pub(in crate::api) use creation::*;
pub(in crate::api) use handlers::*;
pub(super) use task_deletion::{delete_loaded_task_with_cleanup, delete_task};
pub(super) use task_title::update_task_title;

use super::errors::ApiErrorResp;
use crate::daemon::scheduler::SchedulerCommand;
#[cfg(test)]
use crate::daemon::DaemonHandle;
#[cfg(test)]
use crate::daemon::DaemonState;
use crate::daemon::{ProvidersHandle, SessionsHandle, TransportHandle, WorkspacesHandle};
use ctx_core::ids::{RunId, SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
#[cfg(test)]
use ctx_core::models::SandboxBinding;
use ctx_core::models::{
    ExecutionEnvironment, Message, MessageDelivery, Session, SessionEventType, Task, TaskDeltaKind,
    VcsKind, Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor, Worktree,
};
use ctx_observability::logs;
use ctx_settings_model::ExecutionSettings;
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

#[derive(Clone)]
pub(super) struct TaskApiHandles {
    pub(super) sessions: SessionsHandle,
    pub(super) providers: ProvidersHandle,
    pub(super) workspaces: WorkspacesHandle,
    pub(super) transport: TransportHandle,
}

impl TaskApiHandles {
    pub(super) fn new(
        sessions: SessionsHandle,
        providers: ProvidersHandle,
        workspaces: WorkspacesHandle,
        transport: TransportHandle,
    ) -> Self {
        Self {
            sessions,
            providers,
            workspaces,
            transport,
        }
    }
}

#[cfg(test)]
pub(super) fn task_api_states(
    state: &Arc<DaemonState>,
) -> (
    State<SessionsHandle>,
    State<ProvidersHandle>,
    State<WorkspacesHandle>,
    State<TransportHandle>,
) {
    let handle = DaemonHandle::new(Arc::clone(state));
    (
        State(handle.sessions()),
        State(handle.providers()),
        State(handle.workspaces()),
        State(handle.transport()),
    )
}

fn task_request_matches(existing: &Task, title: &str, description: &Option<String>) -> bool {
    existing.title == title && existing.description.as_deref() == description.as_deref()
}
#[cfg(test)]
mod cleanup_lifecycle_tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod storage_admission_http_tests;

#[cfg(test)]
mod tests;
