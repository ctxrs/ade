use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_daemon::daemon::{RouteFileDownloadError, TextRouteDownload, WorkspacesHandle};

pub(in crate::api) async fn get_merge_queue_entry_logs(
    State(state): State<WorkspacesHandle>,
    Path((workspace_id, id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let entry_id =
        MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let download = state
        .download_merge_queue_entry_logs_for_route(workspace_id, entry_id)
        .await
        .map_err(map_route_file_error)?;
    Ok(text_download_response(download))
}

pub(in crate::api::merge_queue_api) fn text_download_response(
    download: TextRouteDownload,
) -> Response {
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
    resp
}

pub(in crate::api::merge_queue_api) fn map_route_file_error(
    error: RouteFileDownloadError,
) -> StatusCode {
    match error {
        RouteFileDownloadError::NotFound => StatusCode::NOT_FOUND,
        RouteFileDownloadError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
