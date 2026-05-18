#[cfg(test)]
use std::path::Path as StdPath;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

#[path = "tasks/creation.rs"]
mod creation;
mod handlers;
#[path = "tasks/task_deletion.rs"]
mod task_deletion;
#[path = "tasks/task_title.rs"]
mod task_title;
pub(in crate::api) use creation::*;
pub(in crate::api) use handlers::*;
pub(super) use task_deletion::delete_task;
pub(super) use task_title::update_task_title;

use super::errors::ApiErrorResp;
#[cfg(test)]
use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
#[cfg(test)]
use ctx_core::models::{ExecutionEnvironment, Workspace, Worktree};
use ctx_daemon::daemon::tasks::{
    ArchiveTaskRouteResponse, CreateTaskRouteRequest, CreateTaskSessionRouteRequest,
    ListWorkspaceArchivedTasksRouteParams, ListWorkspaceArchivedTasksRouteRequest,
    ListWorkspaceTasksRouteParams, SessionRouteResponse, TaskRouteError, TaskRouteErrorKind,
    TaskRouteParams, TaskRouteResponse, UpdateTaskTitleRouteRequest,
    WorkspaceArchivedPageRouteResponse,
};
use ctx_daemon::daemon::{
    ProvidersHandle, SessionsHandle, TasksHandle, TransportHandle, WorkspacesHandle,
};
#[cfg(test)]
use ctx_daemon::test_support::TestDaemon;

#[cfg(test)]
pub(super) fn task_api_task_state(daemon: &TestDaemon) -> State<TasksHandle> {
    State(daemon.handle().tasks())
}

fn task_route_status(error: &TaskRouteError) -> StatusCode {
    match error.kind() {
        TaskRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        TaskRouteErrorKind::NotFound => StatusCode::NOT_FOUND,
        TaskRouteErrorKind::Conflict => StatusCode::CONFLICT,
        TaskRouteErrorKind::Forbidden => StatusCode::FORBIDDEN,
        TaskRouteErrorKind::InsufficientStorage => StatusCode::INSUFFICIENT_STORAGE,
        TaskRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn task_route_api_error(error: TaskRouteError) -> (StatusCode, Json<ApiErrorResp>) {
    (
        task_route_status(&error),
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}
#[cfg(test)]
mod cleanup_lifecycle_tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod storage_admission_http_tests;

#[cfg(test)]
mod tests;
