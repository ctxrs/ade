use super::*;

pub(crate) async fn mcp_spawn_agent(
    State(state): State<Arc<AppState>>,
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

    crate::daemon::sessions::subagents::spawn_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}
