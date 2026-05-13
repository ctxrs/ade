use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;

use crate::api::shared::{map_file_completions_error, FileCompletionsQuery};
use crate::daemon::AppState;

pub(in crate::api) async fn workspace_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    crate::daemon::workspaces::complete_files_for_workspace(&state, ws_id, q.query, q.limit)
        .await
        .map(Json)
        .map_err(map_file_completions_error)
}
