use super::*;
use axum::extract::Extension;
use ctx_daemon::daemon::sessions::subagents::{SpawnAgentReq, SpawnAgentResp};

pub(crate) async fn mcp_spawn_agent(
    State(state): State<SessionsHandle>,
    mcp_auth: Option<Extension<ctx_mcp_auth::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<SpawnAgentReq>,
) -> Result<Json<SpawnAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if let Some(Extension(mcp_auth)) = mcp_auth {
        state
            .require_scoped_mcp_session_context(mcp_auth, parent_id)
            .await
            .map_err(scoped_mcp_session_error)?;
    }

    state
        .spawn_agent(parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

fn scoped_mcp_session_error(
    error: ctx_daemon::daemon::ScopedMcpSessionAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        ctx_daemon::daemon::ScopedMcpSessionAccessError::Unauthorized(message) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: message.to_string(),
            }),
        ),
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
