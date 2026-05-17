use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RunArchiveBatchQuery {
    #[serde(default)]
    max_items: Option<u32>,
}

impl RunArchiveBatchQuery {
    pub(super) fn max_items(self) -> Option<u32> {
        self.max_items
    }
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
