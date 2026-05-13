use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;

pub(super) fn parse_workspace_id(
    ws_id: &str,
) -> Result<WorkspaceId, (StatusCode, Json<serde_json::Value>)> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid workspace id",
            })),
        )
    })?))
}
