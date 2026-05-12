use super::*;
use crate::api::validate_scoped_mcp_session_context;
use axum::extract::Extension;

pub(crate) async fn mcp_spawn_agent(
    State(state): State<Arc<AppState>>,
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
        validate_scoped_mcp_session_context(&state, mcp_auth, parent_id).await?;
    }

    crate::daemon::sessions::subagents::spawn_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}
