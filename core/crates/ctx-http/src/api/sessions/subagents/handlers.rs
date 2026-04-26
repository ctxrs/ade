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
        .map(Json)
}

pub(crate) async fn mcp_oracle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<oracle::OracleRequest>,
) -> Result<Json<oracle::OracleResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let workspace_id = state
        .global_store()
        .get_workspace_id_for_session(session_id)
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

    let store = state.store_for_workspace(workspace_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    store
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

    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let cfg = settings.oracle.as_ref().ok_or((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "oracle is not configured".to_string(),
        }),
    ))?;

    let resp = oracle::oracle_one_shot(cfg, req).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    Ok(Json(resp))
}
