use super::*;
use ctx_core::ids::WorkspaceId;

pub(super) type WorkspaceApiResult<T> = Result<T, (StatusCode, Json<ApiErrorResp>)>;

pub(super) fn parse_workspace_id(id: &str) -> WorkspaceApiResult<WorkspaceId> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?))
}
