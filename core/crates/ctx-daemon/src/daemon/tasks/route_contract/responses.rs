pub use ctx_route_contracts::tasks::{
    ArchiveTaskRouteResponse, ExecutionEnvironmentRouteValue, SessionRouteResponse,
    SessionStatusRouteResponse, SessionSummaryRouteResponse, TaskRouteResponse,
    TaskStatusRouteResponse, WorkspaceArchivedPageRouteResponse, WorkspaceIndexCursorRouteResponse,
    WorkspaceTaskSummaryRouteResponse,
};

use super::super::ArchiveTaskOutcome;

impl From<ArchiveTaskOutcome> for ArchiveTaskRouteResponse {
    fn from(outcome: ArchiveTaskOutcome) -> Self {
        Self::from_task(outcome.task, outcome.cleanup_failed)
    }
}
