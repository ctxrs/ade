use super::*;

pub(in crate::api::workspaces::management) async fn update_workspace_merge_queue_config(
    workspaces: &WorkspacesHandle,
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

    let update = workspace_config::MergeQueueConfigUpdate {
        enabled: req.enabled,
        target_branch: req.target_branch,
        verify_commands,
        push_on_success: req.push_on_success,
        push_remote: req.push_remote,
        push_branch: req.push_branch,
        canonical_sync: Some(workspace_config::MergeQueueCanonicalSync::CleanOnly),
    };
    workspaces
        .update_workspace_merge_queue_config(ctx.workspace_id, update)
        .await
        .map_err(|error| {
            (
                crate::api::shared::status_code_for_request_or_policy_error(&error),
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;

    Ok(UpdateWorkspaceConfigResp { ok: true })
}

pub(in crate::api::workspaces::management) async fn load_workspace_merge_queue_config(
    workspaces: &WorkspacesHandle,
    ctx: &WorkspaceRequestContext,
) -> WorkspaceApiResult<WorkspaceMergeQueueConfigResp> {
    let cfg = workspaces
        .load_workspace_merge_queue_config(ctx.workspace_id)
        .await
        .map_err(workspace_store_api_error)?;

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
