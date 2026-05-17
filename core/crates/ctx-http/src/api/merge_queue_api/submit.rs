use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_daemon::daemon::merge_queue::{
    MergeQueueSubmitRouteError, MergeQueueSubmitRouteErrorKind, SubmitMergeQueueEntryRouteRequest,
};

use crate::api::errors::ApiErrorResp;
use ctx_daemon::daemon::WorkspacesHandle;

pub(in crate::api) async fn submit_merge_queue_entry(
    State(workspaces): State<WorkspacesHandle>,
    mcp_auth: Option<Extension<ctx_mcp_auth::McpAuthContext>>,
    Json(req): Json<SubmitMergeQueueEntryRouteRequest>,
) -> Result<Json<ctx_core::models::MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let entry = workspaces
        .submit_merge_queue_entry_for_route(req, mcp_auth.map(|Extension(auth)| auth))
        .await
        .map_err(merge_queue_submit_route_error)?;
    Ok(Json(entry))
}

fn merge_queue_submit_route_error(
    error: MergeQueueSubmitRouteError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        MergeQueueSubmitRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        MergeQueueSubmitRouteErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
        MergeQueueSubmitRouteErrorKind::NotFound => StatusCode::NOT_FOUND,
        MergeQueueSubmitRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}
