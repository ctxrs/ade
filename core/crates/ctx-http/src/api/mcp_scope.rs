use super::*;
use ctx_daemon::daemon::SessionsHandle;

pub(in crate::api) async fn validate_scoped_mcp_session_context(
    state: &SessionsHandle,
    mcp_auth: ctx_mcp_auth::McpAuthContext,
    session_id: SessionId,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    state
        .require_scoped_mcp_session_context(mcp_auth, session_id)
        .await
        .map_err(scoped_mcp_session_error)
}

fn scoped_mcp_session_error(
    error: ctx_daemon::daemon::ScopedMcpSessionAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        ctx_daemon::daemon::ScopedMcpSessionAccessError::Unauthorized(message) => {
            scoped_mcp_unauthorized(message)
        }
        ctx_daemon::daemon::ScopedMcpSessionAccessError::SessionNotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ),
        ctx_daemon::daemon::ScopedMcpSessionAccessError::StoreUnavailable(error) => (
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
