use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_workspace_config as workspace_config;
use serde::{Deserialize, Serialize};

use super::common;
use crate::api::errors::ApiErrorResp;
use crate::daemon::WorkspacesHandle;

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

pub(in crate::api) async fn get_agent_system_prompt(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<AgentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = common::parse_workspace_id(&id)?;
    let cfg = workspaces
        .load_agent_system_prompt_append(workspace_id)
        .await
        .map_err(common::workspace_store_error)?;

    Ok(Json(agent_response(&cfg)))
}

pub(in crate::api) async fn update_agent_system_prompt(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentSystemPromptConfigReq>,
) -> Result<Json<AgentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = common::parse_workspace_id(&id)?;
    let cfg = workspaces
        .update_agent_system_prompt_append(workspace_id, req.system_prompt_append)
        .await
        .map_err(common::workspace_store_error)?;

    Ok(Json(agent_response(&cfg)))
}

fn agent_response(
    cfg: &workspace_config::AgentSystemPromptAppendConfig,
) -> AgentSystemPromptConfigResponse {
    AgentSystemPromptConfigResponse {
        default_append: cfg.default_append.clone(),
        configured_append: common::configured_append(&cfg.configured_append),
        effective_append: cfg.effective_append(),
        source: common::source_label(cfg.source()),
    }
}
