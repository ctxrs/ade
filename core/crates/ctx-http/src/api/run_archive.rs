use super::*;
use ctx_daemon::daemon::workspaces::RunArchiveIngestError;
use ctx_daemon::daemon::WorkspacesHandle;

mod validation;
use validation::{
    parse_archive_run_id, parse_archive_workspace_id, requested_batch_item_limit,
    run_archive_api_error, RunArchiveBatchQuery,
};

pub(super) async fn build_workspace_run_archive_ingest_batch(
    State(state): State<WorkspacesHandle>,
    Path((workspace_id, run_id)): Path<(String, String)>,
    Query(query): Query<RunArchiveBatchQuery>,
) -> Result<Json<Option<RunArchiveIngestBatch>>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_archive_workspace_id(&workspace_id)?;
    let run_id = parse_archive_run_id(&run_id)?;
    let max_items = requested_batch_item_limit(query)?;
    let batch = state
        .build_run_archive_ingest_batch(workspace_id, run_id, max_items)
        .await
        .map_err(|err| run_archive_ingest_api_error("build", err))?;

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
    let max_items = requested_batch_item_limit(query)?;
    if batch.run.workspace_id != workspace_id {
        return Err(run_archive_api_error(
            StatusCode::BAD_REQUEST,
            "archive ingest batch workspace_id must match route workspace id",
        ));
    }
    if batch.run.id != run_id {
        return Err(run_archive_api_error(
            StatusCode::BAD_REQUEST,
            "archive ingest batch run id must match route run id",
        ));
    }
    if batch.run.org_id.is_none() || !batch.scope.is_cloud_visible() {
        return Err(run_archive_api_error(
            StatusCode::BAD_REQUEST,
            "archive ingest acknowledgement requires an org-visible batch",
        ));
    }

    state
        .acknowledge_run_archive_ingest_batch(workspace_id, run_id, max_items, batch)
        .await
        .map(Json)
        .map_err(|err| run_archive_ingest_api_error("acknowledge", err))
}

fn run_archive_ingest_api_error(
    action: &'static str,
    error: RunArchiveIngestError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        RunArchiveIngestError::WorkspaceNotFound => run_archive_api_error(
            StatusCode::NOT_FOUND,
            "workspace not found for run archive ingest",
        ),
        RunArchiveIngestError::AcknowledgementConflict(message) => {
            run_archive_api_error(StatusCode::CONFLICT, message)
        }
        RunArchiveIngestError::Internal(err) => run_archive_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to {action} run archive ingest batch: {err:#}"),
        ),
    }
}
