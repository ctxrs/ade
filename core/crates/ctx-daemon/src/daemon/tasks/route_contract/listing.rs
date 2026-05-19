use chrono::{DateTime, Utc};
use ctx_core::models::WorkspaceIndexCursor;
use serde::Deserialize;

use crate::daemon::TasksHandle;

use super::common::{parse_task_id, parse_workspace_id, TaskRouteError};
use super::responses::{TaskRouteResponse, WorkspaceArchivedPageRouteResponse};

#[derive(Debug)]
pub struct ListWorkspaceTasksRouteParams {
    workspace_id: String,
}

impl ListWorkspaceTasksRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListWorkspaceArchivedTasksRouteRequest {
    limit: Option<u32>,
    cursor_sort_at: Option<String>,
    cursor_task_id: Option<String>,
}

#[derive(Debug)]
pub struct ListWorkspaceArchivedTasksRouteParams {
    workspace_id: String,
    query: ListWorkspaceArchivedTasksRouteRequest,
}

impl ListWorkspaceArchivedTasksRouteParams {
    pub fn new(
        workspace_id: impl Into<String>,
        query: ListWorkspaceArchivedTasksRouteRequest,
    ) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            query,
        }
    }
}

impl TasksHandle {
    pub async fn list_workspace_tasks_for_route(
        &self,
        params: ListWorkspaceTasksRouteParams,
    ) -> Result<Vec<TaskRouteResponse>, TaskRouteError> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        self.list_workspace_tasks(workspace_id)
            .await
            .map(|tasks| tasks.into_iter().map(TaskRouteResponse::from).collect())
            .map_err(TaskRouteError::from_workspace_store)
    }

    pub async fn list_workspace_archived_page_for_route(
        &self,
        params: ListWorkspaceArchivedTasksRouteParams,
    ) -> Result<WorkspaceArchivedPageRouteResponse, TaskRouteError> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let limit = params.query.limit.unwrap_or(50) as i64;
        let cursor = parse_archived_cursor(
            params.query.cursor_sort_at.as_deref(),
            params.query.cursor_task_id.as_deref(),
        )?;
        self.list_workspace_archived_page(workspace_id, cursor, limit)
            .await
            .map(WorkspaceArchivedPageRouteResponse::from)
            .map_err(TaskRouteError::from_workspace_store)
    }
}

pub(super) fn parse_archived_cursor(
    cursor_sort_at: Option<&str>,
    cursor_task_id: Option<&str>,
) -> Result<Option<WorkspaceIndexCursor>, TaskRouteError> {
    match (cursor_sort_at, cursor_task_id) {
        (None, None) => Ok(None),
        (Some(sort_at), Some(task_id)) => {
            let sort_at = DateTime::parse_from_rfc3339(sort_at)
                .map_err(|_| TaskRouteError::bad_request("invalid cursor"))?
                .with_timezone(&Utc);
            let task_id = parse_task_id(task_id)?;
            Ok(Some(WorkspaceIndexCursor { sort_at, task_id }))
        }
        _ => Err(TaskRouteError::bad_request("invalid cursor")),
    }
}
