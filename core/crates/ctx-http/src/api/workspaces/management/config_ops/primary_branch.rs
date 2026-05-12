use super::*;

pub(in crate::api::workspaces::management) async fn load_workspace_primary_branch(
    store: &ctx_store::Store,
) -> WorkspaceApiResult<WorkspacePrimaryBranchResp> {
    let primary_branch = workspace_config::load_primary_branch(store)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace primary branch is not configured".to_string(),
            }),
        ))?;
    Ok(WorkspacePrimaryBranchResp { primary_branch })
}

pub(in crate::api::workspaces::management) async fn update_workspace_primary_branch_config(
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: UpdateWorkspacePrimaryBranchReq,
) -> WorkspaceApiResult<WorkspacePrimaryBranchResp> {
    let primary_branch =
        ctx_workspace_services::workspace_registration::validate_workspace_primary_branch(
            StdPath::new(&ctx.workspace.root_path),
            &req.primary_branch,
        )
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(error.message()),
                }),
            )
        })?;
    workspace_config::update_primary_branch(&ctx.store, &primary_branch)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    let worktrees = ctx
        .store
        .list_worktrees(ctx.workspace_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    for worktree in worktrees {
        if let Err(error) = emit_worktree_vcs_snapshot_for_worktree(state, &worktree, true).await {
            tracing::warn!(
                workspace_id = %ctx.workspace_id.0,
                worktree_id = %worktree.id.0,
                "failed to refresh worktree vcs after primary branch update: {error:#}"
            );
        }
    }
    Ok(WorkspacePrimaryBranchResp { primary_branch })
}
