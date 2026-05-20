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
use common::route_error_kind_for_internal_error;
#[cfg(test)]
mod tests;
