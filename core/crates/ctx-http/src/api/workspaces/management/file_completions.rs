use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use crate::api::workspaces::{workspace_route_status, WorkspaceRouteParams};
use ctx_daemon::daemon::{WorkspaceFileCompletionsRouteQuery, WorkspacesHandle};

pub(in crate::api) async fn workspace_file_completions(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Query(q): Query<WorkspaceFileCompletionsRouteQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    workspaces
        .workspace_file_completions_for_route(WorkspaceRouteParams::new(id), q)
        .await
        .map(Json)
        .map_err(|error| workspace_route_status(&error))
}
