use std::path::{Path as StdPath, PathBuf};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};

use crate::api::shared::path_resolves_within_root;
use crate::daemon::WorkspacesHandle;

pub(in crate::api) async fn get_merge_queue_entry_logs(
    State(state): State<WorkspacesHandle>,
    Path((workspace_id, id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let entry_id =
        MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .get_workspace_merge_queue_entry(workspace_id, entry_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let (workspace, run) = state
        .latest_merge_queue_run_for_route(workspace_id, entry_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(path) = run.log_path.as_deref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let log_root = PathBuf::from(&workspace.root_path)
        .join(".ctx")
        .join("merge-queue")
        .join("logs");
    if !path_resolves_within_root(StdPath::new(path), &log_root).await {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let filename = format!("merge-queue-{}.log", entry_id.0);
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}
