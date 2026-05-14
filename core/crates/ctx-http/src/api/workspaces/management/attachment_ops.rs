use super::*;

pub(super) async fn create_and_sync_workspace_attachment(
    workspaces: &WorkspacesHandle,
    ctx: &WorkspaceRequestContext,
    req: CreateWorkspaceAttachmentReq,
) -> WorkspaceApiResult<Vec<WorkspaceAttachment>> {
    let cfg = AttachmentConfig {
        kind: req.kind,
        name: req.name,
        source: req.source,
        revision: req.revision,
        subpath: req.subpath,
        mount_relpath: req.mount_relpath,
        mode: req.mode,
        update_policy: req.update_policy,
    };
    workspaces
        .upsert_workspace_attachment(ctx.workspace_id, cfg)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;

    workspaces
        .sync_workspace_attachments(&ctx.workspace, true)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })
}

pub(super) async fn delete_and_sync_workspace_attachment(
    workspaces: &WorkspacesHandle,
    ctx: &WorkspaceRequestContext,
    req: DeleteWorkspaceAttachmentReq,
) -> WorkspaceApiResult<Vec<WorkspaceAttachment>> {
    let removed = workspaces
        .delete_workspace_attachment(ctx.workspace_id, req.kind, &req.name)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    if !removed {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "attachment not found".to_string(),
            }),
        ));
    }

    workspaces
        .sync_workspace_attachments(&ctx.workspace, false)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })
}
