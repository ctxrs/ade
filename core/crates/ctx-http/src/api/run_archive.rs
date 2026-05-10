use super::*;

mod validation;
use validation::{
    parse_archive_run_id, parse_archive_workspace_id, requested_batch_item_limit,
    run_archive_api_error, RunArchiveBatchQuery,
};

pub(super) async fn build_workspace_run_archive_ingest_batch(
    State(state): State<Arc<AppState>>,
    Path((workspace_id, run_id)): Path<(String, String)>,
    Query(query): Query<RunArchiveBatchQuery>,
) -> Result<Json<Option<RunArchiveIngestBatch>>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_archive_workspace_id(&workspace_id)?;
    let run_id = parse_archive_run_id(&run_id)?;
    let max_items = requested_batch_item_limit(query)?;
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|err| {
            run_archive_api_error(
                StatusCode::NOT_FOUND,
                format!("workspace not found for run archive ingest: {err:#}"),
            )
        })?;

    let batch = store
        .build_run_archive_ingest_batch(run_id, max_items)
        .await
        .map_err(|err| {
            run_archive_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build run archive ingest batch: {err:#}"),
            )
        })?;

    Ok(Json(
        batch.filter(|batch| batch.run.workspace_id == workspace_id),
    ))
}

pub(super) async fn acknowledge_workspace_run_archive_ingest_batch(
    State(state): State<Arc<AppState>>,
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

    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|err| {
            run_archive_api_error(
                StatusCode::NOT_FOUND,
                format!("workspace not found for run archive ingest: {err:#}"),
            )
        })?;
    let cursor = store
        .get_run_archive_ingest_cursor(run_id)
        .await
        .map_err(|err| {
            run_archive_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load run archive ingest cursor: {err:#}"),
            )
        })?;
    let current_watermark = cursor
        .as_ref()
        .map(|cursor| cursor.watermark)
        .unwrap_or_default();
    if batch.from != current_watermark {
        return Err(run_archive_api_error(
            StatusCode::CONFLICT,
            "archive ingest acknowledgement is stale for the current cursor",
        ));
    }

    let Some(mut expected_batch) = store
        .build_run_archive_ingest_batch_after(run_id, batch.from, max_items, cursor.is_none())
        .await
        .map_err(|err| {
            run_archive_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to verify run archive ingest batch: {err:#}"),
            )
        })?
    else {
        return Err(run_archive_api_error(
            StatusCode::CONFLICT,
            "archive ingest acknowledgement does not match an available batch",
        ));
    };
    expected_batch.created_at = batch.created_at;
    if expected_batch != batch {
        return Err(run_archive_api_error(
            StatusCode::CONFLICT,
            "archive ingest acknowledgement does not match the current batch",
        ));
    }

    store
        .acknowledge_run_archive_ingest_batch(&batch)
        .await
        .map(Json)
        .map_err(|err| {
            run_archive_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to acknowledge run archive ingest batch: {err:#}"),
            )
        })
}
