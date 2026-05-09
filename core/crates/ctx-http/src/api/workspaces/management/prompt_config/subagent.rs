use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_workspace_config as workspace_config;
use serde::{Deserialize, Serialize};

use super::common;
use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;

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

pub(in crate::api) async fn get_subagent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SubagentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let store = common::store_for_existing_workspace(&state, &id).await?;
    let cfg = workspace_config::load_subagent_system_prompt_append(&store)
        .await
        .map_err(common::bad_request)?;

    Ok(Json(subagent_response(&cfg)))
}

pub(in crate::api) async fn update_subagent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSubagentSystemPromptConfigReq>,
) -> Result<Json<SubagentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let store = common::store_for_existing_workspace(&state, &id).await?;
    workspace_config::update_subagent_system_prompt_append(&store, req.system_prompt_append)
        .await
        .map_err(common::bad_request)?;
    let cfg = workspace_config::load_subagent_system_prompt_append(&store)
        .await
        .map_err(common::bad_request)?;

    Ok(Json(subagent_response(&cfg)))
}

fn subagent_response(
    cfg: &workspace_config::SubagentSystemPromptAppendConfig,
) -> SubagentSystemPromptConfigResponse {
    SubagentSystemPromptConfigResponse {
        default_append: cfg.default_append.clone(),
        configured_append: common::configured_append(&cfg.configured_append),
        effective_append: cfg.effective_append(),
        source: common::source_label(cfg.source()),
    }
}
