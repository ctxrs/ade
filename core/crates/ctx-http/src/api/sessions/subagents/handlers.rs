use super::*;

pub(crate) async fn mcp_send_input(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SendInputReq>,
) -> Result<Json<SendInputResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    crate::daemon::sessions::subagents::send_input(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_archive_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ArchiveAgentReq>,
) -> Result<Json<ArchiveAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    crate::daemon::sessions::subagents::archive_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_list_agents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<AgentSummary>>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    crate::daemon::sessions::subagents::list_agents(state, parent_id)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<GetAgentReq>,
) -> Result<Json<GetAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    crate::daemon::sessions::subagents::get_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_interrupt_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<InterruptAgentReq>,
) -> Result<Json<InterruptAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    crate::daemon::sessions::subagents::interrupt_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_wait_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<WaitAgentReq>,
) -> Result<Json<WaitAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    crate::daemon::sessions::subagents::wait_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}
