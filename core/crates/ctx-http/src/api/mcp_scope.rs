use super::*;

pub(in crate::api) async fn validate_scoped_mcp_session_context(
    state: &Arc<AppState>,
    mcp_auth: ctx_mcp_auth::McpAuthContext,
    session_id: SessionId,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    if mcp_auth.session_id != session_id {
        return Err(scoped_mcp_unauthorized(
            "scoped ctx-mcp token is limited to the current session",
        ));
    }

    let store = state.store_for_session(session_id).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    if session.workspace_id != mcp_auth.workspace_id || session.worktree_id != mcp_auth.worktree_id
    {
        return Err(scoped_mcp_unauthorized(
            "scoped ctx-mcp token does not match the loaded session scope",
        ));
    }

    Ok(())
}

pub(in crate::api) fn scoped_mcp_unauthorized(message: &str) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiErrorResp {
            error: message.to_string(),
        }),
    )
}
