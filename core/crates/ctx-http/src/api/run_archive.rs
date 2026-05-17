use super::*;
use ctx_daemon::daemon::workspaces::{
    AcknowledgeRunArchiveIngestBatchRouteRequest, BuildRunArchiveIngestBatchRouteRequest,
    RunArchiveRouteError, RunArchiveRouteErrorKind,
};
use ctx_daemon::daemon::WorkspacesHandle;

mod validation;
use validation::{
    parse_archive_run_id, parse_archive_workspace_id, run_archive_api_error, RunArchiveBatchQuery,
};

pub(super) async fn build_workspace_run_archive_ingest_batch(
    State(state): State<WorkspacesHandle>,
    Path((workspace_id, run_id)): Path<(String, String)>,
    Query(query): Query<RunArchiveBatchQuery>,
) -> Result<Json<Option<RunArchiveIngestBatch>>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_archive_workspace_id(&workspace_id)?;
    let run_id = parse_archive_run_id(&run_id)?;
    let batch = state
        .build_run_archive_ingest_batch_for_route(BuildRunArchiveIngestBatchRouteRequest {
            workspace_id,
            run_id,
            max_items: query.max_items(),
        })
        .await
        .map_err(run_archive_route_error)?;

    Ok(Json(batch))
}

pub(super) async fn acknowledge_workspace_run_archive_ingest_batch(
    State(state): State<WorkspacesHandle>,
    Path((workspace_id, run_id)): Path<(String, String)>,
    Query(query): Query<RunArchiveBatchQuery>,
    Json(batch): Json<RunArchiveIngestBatch>,
) -> Result<Json<RunArchiveIngestCursor>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_archive_workspace_id(&workspace_id)?;
    let run_id = parse_archive_run_id(&run_id)?;

    state
        .acknowledge_run_archive_ingest_batch_for_route(
            AcknowledgeRunArchiveIngestBatchRouteRequest {
                workspace_id,
                run_id,
                max_items: query.max_items(),
                batch,
            },
        )
        .await
        .map(Json)
        .map_err(run_archive_route_error)
}

fn run_archive_route_error(error: RunArchiveRouteError) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        RunArchiveRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        RunArchiveRouteErrorKind::NotFound => StatusCode::NOT_FOUND,
        RunArchiveRouteErrorKind::Conflict => StatusCode::CONFLICT,
        RunArchiveRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    run_archive_api_error(status, error.message())
}
