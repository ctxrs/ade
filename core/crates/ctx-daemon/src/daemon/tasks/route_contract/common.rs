use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_observability::logs;

use crate::daemon::WorkspaceStoreAccessError;

use super::super::{TaskCreateError, TaskLifecycleError, TaskSessionCreateError};

#[derive(Debug)]
pub struct TaskRouteParams {
    pub(super) task_id: String,
}

impl TaskRouteParams {
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskRouteErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Forbidden,
    InsufficientStorage,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRouteError {
    kind: TaskRouteErrorKind,
    message: String,
}

impl TaskRouteError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::Conflict,
            message: message.into(),
        }
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    pub(super) fn classified_internal(error: &anyhow::Error, message: impl Into<String>) -> Self {
        Self {
            kind: route_error_kind_for_internal_error(error),
            message: message.into(),
        }
    }

    pub fn kind(&self) -> TaskRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(super) fn from_workspace_store(error: WorkspaceStoreAccessError) -> Self {
        match error {
            WorkspaceStoreAccessError::NotFound => Self::not_found("workspace not found"),
            WorkspaceStoreAccessError::Unavailable(error) => {
                tracing::warn!("workspace store unavailable for task route: {error:#}");
                Self::internal("workspace store unavailable")
            }
        }
    }

    pub(super) fn from_task_create(error: TaskCreateError) -> Self {
        match error {
            TaskCreateError::BadRequest(error) => Self::bad_request(error),
            TaskCreateError::NotFound(error) => Self::not_found(error),
            TaskCreateError::Conflict(error) => Self::conflict(error),
            TaskCreateError::Internal(error) => {
                let message = logs::redact_sensitive(&error.to_string());
                Self::classified_internal(&error, message)
            }
            TaskCreateError::DefaultSessionFailed(error) => {
                let kind = Self::from_task_session_create(error).kind;
                Self {
                    kind,
                    message: "failed to create default session".to_string(),
                }
            }
            TaskCreateError::DefaultSessionConflict => {
                Self::conflict("task id already exists with a different default session")
            }
        }
    }

    pub(super) fn from_task_session_create(error: TaskSessionCreateError) -> Self {
        match error {
            TaskSessionCreateError::BadRequest => Self::bad_request("bad request"),
            TaskSessionCreateError::NotFound => Self::not_found("task not found"),
            TaskSessionCreateError::Conflict => Self::conflict("session conflict"),
            TaskSessionCreateError::Internal(error) => {
                tracing::warn!("task session creation failed: {error:#}");
                let message = logs::redact_sensitive(&error.to_string());
                Self::classified_internal(&error, message)
            }
        }
    }

    pub(super) fn from_task_lifecycle(error: TaskLifecycleError) -> Self {
        match error {
            TaskLifecycleError::NotFound => Self::not_found("task not found"),
            TaskLifecycleError::Internal(error) => {
                tracing::warn!("task lifecycle operation failed: {error:#}");
                Self::classified_internal(&error, "task lifecycle operation failed")
            }
        }
    }
}

pub(super) fn parse_workspace_id(value: &str) -> Result<WorkspaceId, TaskRouteError> {
    uuid::Uuid::parse_str(value)
        .map(WorkspaceId)
        .map_err(|_| TaskRouteError::bad_request("invalid workspace id"))
}

pub(super) fn parse_task_id(value: &str) -> Result<TaskId, TaskRouteError> {
    uuid::Uuid::parse_str(value)
        .map(TaskId)
        .map_err(|_| TaskRouteError::bad_request("invalid task id"))
}

pub(super) fn route_error_kind_for_internal_error(error: &anyhow::Error) -> TaskRouteErrorKind {
    if ctx_settings_service::is_execution_policy_denial(error) {
        TaskRouteErrorKind::Forbidden
    } else if error
        .chain()
        .any(|cause| ctx_storage_admission::is_storage_exhaustion_error(&cause.to_string()))
    {
        TaskRouteErrorKind::InsufficientStorage
    } else {
        TaskRouteErrorKind::Internal
    }
}
