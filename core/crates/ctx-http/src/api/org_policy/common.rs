use super::*;

pub(super) fn policy_api_error(
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

pub(super) fn parse_org_id(raw: &str) -> Result<OrgId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(raw)
        .map(OrgId)
        .map_err(|_| policy_api_error(StatusCode::BAD_REQUEST, "invalid org id"))
}

pub(super) fn parse_workspace_id(
    raw: &str,
) -> Result<WorkspaceId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(raw)
        .map(WorkspaceId)
        .map_err(|_| policy_api_error(StatusCode::BAD_REQUEST, "invalid workspace id"))
}
