use super::*;

pub(in crate::api) async fn validate_scoped_mcp_session_context(
    state: &Arc<AppState>,
    mcp_auth: ctx_mcp_auth::McpAuthContext,
    session_id: SessionId,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::require_scoped_mcp_session_context(state, mcp_auth, session_id)
        .await
        .map_err(scoped_mcp_session_error)
}

fn scoped_mcp_session_error(
    error: crate::daemon::ScopedMcpSessionAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::ScopedMcpSessionAccessError::Unauthorized(message) => {
            scoped_mcp_unauthorized(message)
        }
        crate::daemon::ScopedMcpSessionAccessError::SessionNotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ),
        crate::daemon::ScopedMcpSessionAccessError::StoreUnavailable(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        ),
    }
}

fn scoped_mcp_unauthorized(message: &str) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiErrorResp {
            error: message.to_string(),
        }),
    )
}
