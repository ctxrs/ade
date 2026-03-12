use super::*;

#[derive(Debug, Serialize)]
pub(in crate::api) struct AgentSystemPromptConfigResponse {
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateAgentSystemPromptConfigReq {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct SubagentSystemPromptConfigResponse {
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateSubagentSystemPromptConfigReq {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

pub(in crate::api) async fn get_agent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AgentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
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
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let cfg = workspace_config::load_agent_system_prompt_append(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = AgentSystemPromptConfigResponse {
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}

pub(in crate::api) async fn update_agent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentSystemPromptConfigReq>,
) -> Result<Json<AgentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
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
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    workspace_config::update_agent_system_prompt_append(&store, req.system_prompt_append)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let cfg = workspace_config::load_agent_system_prompt_append(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = AgentSystemPromptConfigResponse {
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}

pub(in crate::api) async fn get_subagent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SubagentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
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
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let cfg = workspace_config::load_subagent_system_prompt_append(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = SubagentSystemPromptConfigResponse {
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}

pub(in crate::api) async fn update_subagent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSubagentSystemPromptConfigReq>,
) -> Result<Json<SubagentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
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
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    workspace_config::update_subagent_system_prompt_append(&store, req.system_prompt_append)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let cfg = workspace_config::load_subagent_system_prompt_append(&store)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = SubagentSystemPromptConfigResponse {
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}
