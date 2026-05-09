use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateWorktreeBootstrapReq {
    #[serde(default)]
    setup_command: Option<String>,
    #[serde(default)]
    timeout_sec: Option<u64>,
    #[serde(default)]
    wait_for_completion: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct WorkspaceWorktreeBootstrapConfigResp {
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_for_completion: Option<bool>,
}

pub(in crate::api) async fn get_worktree_bootstrap_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceWorktreeBootstrapConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    let cfg = workspace_config::load_worktree_bootstrap_config(&ctx.store)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;

    Ok(Json(WorkspaceWorktreeBootstrapConfigResp {
        setup_command: cfg.as_ref().and_then(|value| value.setup_command.clone()),
        timeout_sec: cfg.as_ref().and_then(|value| value.timeout_sec),
        wait_for_completion: cfg.as_ref().and_then(|value| value.wait_for_completion),
    }))
}

pub(in crate::api) async fn update_worktree_bootstrap_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorktreeBootstrapReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    workspace_config::update_worktree_bootstrap_config(
        &ctx.store,
        workspace_config::WorktreeBootstrapConfigUpdate {
            setup_command: req.setup_command,
            timeout_sec: req.timeout_sec,
            wait_for_completion: req.wait_for_completion,
        },
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    Ok(Json(UpdateWorkspaceConfigResp { ok: true }))
}
