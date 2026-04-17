use super::*;

pub(crate) async fn mcp_agent_init(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentInitReq>,
) -> Result<Json<AgentInitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    crate::daemon::sessions::subagents::init_subagents(state, parent_id, req)
        .await
        .map(Json)
}
