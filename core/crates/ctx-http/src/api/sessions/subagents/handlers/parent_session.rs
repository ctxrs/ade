use super::*;
use crate::api::validate_scoped_mcp_session_context;
use axum::extract::Extension;

pub(super) async fn resolve_scoped_parent_session_id(
    state: &Arc<AppState>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    id: String,
) -> Result<SessionId, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if let Some(Extension(mcp_auth)) = mcp_auth {
        validate_scoped_mcp_session_context(state, mcp_auth, parent_id).await?;
    }

    Ok(parent_id)
}
