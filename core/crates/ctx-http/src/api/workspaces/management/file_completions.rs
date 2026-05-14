use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;

use crate::api::shared::{map_file_completions_error, FileCompletionsQuery};
use ctx_daemon::daemon::WorkspacesHandle;

pub(in crate::api) async fn workspace_file_completions(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    workspaces
        .complete_files_for_workspace(ws_id, q.query, q.limit)
        .await
        .map(Json)
        .map_err(map_file_completions_error)
}
