use super::*;
use ctx_core::ids::WorktreeId;

pub(in crate::api) async fn get_worktree(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<WorktreeRouteResponse>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match workspaces.get_worktree_for_route(worktree_id).await {
        Ok(Some(wt)) => Ok(Json(wt)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(error) => Err(workspace_route_status(&error)),
    }
}

pub(in crate::api) async fn get_worktree_bootstrap_logs(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let download = workspaces
        .download_worktree_bootstrap_logs_for_route(worktree_id)
        .await
        .map_err(workspace_route_file_status)?;
    let mut resp = Response::new(Body::from(download.bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{}\"", download.filename))
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

fn workspace_route_file_status(error: ctx_daemon::daemon::RouteFileDownloadError) -> StatusCode {
    match error {
        ctx_daemon::daemon::RouteFileDownloadError::NotFound => StatusCode::NOT_FOUND,
        ctx_daemon::daemon::RouteFileDownloadError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
