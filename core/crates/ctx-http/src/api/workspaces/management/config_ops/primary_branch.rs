use super::*;

pub(in crate::api::workspaces::management) async fn load_workspace_primary_branch(
    workspaces: &WorkspacesHandle,
    ctx: &WorkspaceRequestContext,
) -> WorkspaceApiResult<WorkspacePrimaryBranchResp> {
    let primary_branch = workspaces
        .load_workspace_primary_branch_config(ctx.workspace_id)
        .await
        .map_err(workspace_store_api_error)?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace primary branch is not configured".to_string(),
            }),
        ))?;
    Ok(WorkspacePrimaryBranchResp { primary_branch })
}

pub(in crate::api::workspaces::management) async fn update_workspace_primary_branch_config(
    workspaces: &WorkspacesHandle,
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
    workspaces
        .update_workspace_primary_branch_config(&ctx.workspace, &primary_branch)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    Ok(WorkspacePrimaryBranchResp { primary_branch })
}
