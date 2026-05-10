use super::*;

const DEFAULT_RUN_ARCHIVE_BATCH_ITEMS: u32 = 250;
const MAX_RUN_ARCHIVE_BATCH_ITEMS: u32 = 1_000;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RunArchiveBatchQuery {
    #[serde(default)]
    max_items: Option<u32>,
}

pub(super) fn run_archive_api_error(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

pub(super) fn parse_archive_workspace_id(
    raw: &str,
) -> Result<WorkspaceId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(raw)
        .map(WorkspaceId)
        .map_err(|_| run_archive_api_error(StatusCode::BAD_REQUEST, "invalid workspace id"))
}

pub(super) fn parse_archive_run_id(raw: &str) -> Result<RunId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(raw)
        .map(RunId)
        .map_err(|_| run_archive_api_error(StatusCode::BAD_REQUEST, "invalid run id"))
}

pub(super) fn requested_batch_item_limit(
    query: RunArchiveBatchQuery,
) -> Result<u32, (StatusCode, Json<ApiErrorResp>)> {
    let max_items = query.max_items.unwrap_or(DEFAULT_RUN_ARCHIVE_BATCH_ITEMS);
    if max_items == 0 || max_items > MAX_RUN_ARCHIVE_BATCH_ITEMS {
        return Err(run_archive_api_error(
            StatusCode::BAD_REQUEST,
            format!("max_items must be between 1 and {MAX_RUN_ARCHIVE_BATCH_ITEMS}"),
        ));
    }
    Ok(max_items)
}
