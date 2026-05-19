mod common;
mod creation;
mod lifecycle;
mod listing;
mod responses;

pub use common::{TaskRouteError, TaskRouteErrorKind, TaskRouteParams};
pub use creation::{
    CreateTaskDefaultSessionRouteRequest, CreateTaskRouteRequest, CreateTaskSessionRouteRequest,
};
pub use lifecycle::UpdateTaskTitleRouteRequest;
pub use listing::{
    ListWorkspaceArchivedTasksRouteParams, ListWorkspaceArchivedTasksRouteRequest,
    ListWorkspaceTasksRouteParams,
};
pub use responses::{
    ArchiveTaskRouteResponse, ExecutionEnvironmentRouteValue, SessionRouteResponse,
    SessionStatusRouteResponse, SessionSummaryRouteResponse, TaskRouteResponse,
    TaskStatusRouteResponse, WorkspaceArchivedPageRouteResponse, WorkspaceIndexCursorRouteResponse,
    WorkspaceTaskSummaryRouteResponse,
};

#[cfg(test)]
pub(super) use super::ArchiveTaskOutcome;
#[cfg(test)]
pub(super) use chrono::{DateTime, Utc};
#[cfg(test)]
use common::route_error_kind_for_internal_error;
#[cfg(test)]
pub(super) use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
#[cfg(test)]
pub(super) use ctx_core::models::{
    ExecutionEnvironment, Session, SessionStatus, SessionSummary, Task, TaskStatus,
    WorkspaceArchivedPage, WorkspaceIndexCursor, WorkspaceTaskSummary,
};
#[cfg(test)]
use listing::parse_archived_cursor;

#[cfg(test)]
mod tests;
