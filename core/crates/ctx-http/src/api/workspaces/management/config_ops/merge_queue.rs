use super::*;

pub(in crate::api::workspaces::management) async fn update_workspace_merge_queue_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: UpdateMergeQueueConfigReq,
) -> WorkspaceApiResult<UpdateWorkspaceConfigResp> {
    let verify_commands = req
        .verify_command
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| vec![value])
        .unwrap_or_default();

    let was_enabled = workspace_config::load_merge_queue_config(&ctx.store)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?
        .enabled;
    workspace_config::update_merge_queue_config(
        &ctx.store,
        workspace_config::MergeQueueConfigUpdate {
            enabled: req.enabled,
            target_branch: req.target_branch,
            verify_commands,
            push_on_success: req.push_on_success,
            push_remote: req.push_remote,
            push_branch: req.push_branch,
            canonical_sync: Some(workspace_config::MergeQueueCanonicalSync::CleanOnly),
        },
    )
    .await
    .map_err(|error| {
        (
            crate::api::shared::status_code_for_request_or_policy_error(&error),
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        )
    })?;

    if !was_enabled && req.enabled {
        crate::daemon::merge_queue::schedule_workspace_if_enabled_and_queued(
            state,
            ctx.workspace_id,
        )
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    } else if was_enabled && !req.enabled {
        crate::daemon::merge_queue::cancel_queued_entries_for_disabled_workspace(
            state,
            &ctx.store,
            ctx.workspace_id,
        )
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    }

    Ok(UpdateWorkspaceConfigResp { ok: true })
}

pub(in crate::api::workspaces::management) async fn load_workspace_merge_queue_config(
    store: &ctx_store::Store,
) -> WorkspaceApiResult<WorkspaceMergeQueueConfigResp> {
    let cfg = workspace_config::load_merge_queue_config(store)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;

    let verify_command = cfg.verify_commands.into_iter().next();
    Ok(WorkspaceMergeQueueConfigResp {
        enabled: cfg.enabled,
        target_branch: cfg.target_branch,
        verify_command,
        push_on_success: cfg.push_on_success,
        push_remote: cfg.push_remote,
        push_branch: cfg.push_branch,
    })
}
