#[cfg(test)]
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
pub(in crate::api) use creation::*;
pub(in crate::api) use handlers::*;
pub(super) use task_deletion::delete_task;
pub(super) use task_title::update_task_title;

use super::errors::ApiErrorResp;
#[cfg(test)]
use crate::daemon::DaemonHandle;
#[cfg(test)]
use crate::daemon::DaemonState;
use crate::daemon::{
    ProvidersHandle, SessionsHandle, TasksHandle, TransportHandle, WorkspacesHandle,
};
#[cfg(test)]
use ctx_core::ids::WorktreeId;
use ctx_core::ids::{TaskId, WorkspaceId};
#[cfg(test)]
use ctx_core::models::SandboxBinding;
#[cfg(test)]
use ctx_core::models::Workspace;
#[cfg(test)]
use ctx_core::models::Worktree;
use ctx_core::models::{
    ExecutionEnvironment, Session, Task, WorkspaceArchivedPage, WorkspaceIndexCursor,
};
use ctx_observability::logs;

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

impl CreateTaskReq {
    fn into_create_task_input(
        self,
    ) -> Result<crate::daemon::tasks::CreateTaskInput, (StatusCode, Json<ApiErrorResp>)> {
        let task_id = match self.id.as_deref().map(str::trim) {
            Some("") | None => None,
            Some(raw) => Some(TaskId(uuid::Uuid::parse_str(raw).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "invalid task id".to_string(),
                    }),
                )
            })?)),
        };
        Ok(crate::daemon::tasks::CreateTaskInput {
            task_id,
            title: self.title,
            description: self.description,
            default_session: self
                .default_session
                .map(|default_session| default_session.into_task_session_input(None)),
        })
    }
}

#[cfg(test)]
pub(super) fn task_api_task_state(state: &Arc<DaemonState>) -> State<TasksHandle> {
    State(DaemonHandle::new(Arc::clone(state)).tasks())
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

fn task_create_api_error(
    error: crate::daemon::tasks::TaskCreateError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::tasks::TaskCreateError::BadRequest(error) => {
            (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error }))
        }
        crate::daemon::tasks::TaskCreateError::NotFound(error) => {
            (StatusCode::NOT_FOUND, Json(ApiErrorResp { error }))
        }
        crate::daemon::tasks::TaskCreateError::Conflict(error) => {
            (StatusCode::CONFLICT, Json(ApiErrorResp { error }))
        }
        crate::daemon::tasks::TaskCreateError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        ),
        crate::daemon::tasks::TaskCreateError::DefaultSessionFailed(error) => (
            task_session_create_status(error),
            Json(ApiErrorResp {
                error: "failed to create default session".to_string(),
            }),
        ),
        crate::daemon::tasks::TaskCreateError::DefaultSessionConflict => (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "task id already exists with a different default session".to_string(),
            }),
        ),
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
