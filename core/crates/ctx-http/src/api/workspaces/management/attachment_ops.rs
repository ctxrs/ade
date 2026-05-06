use super::*;

pub(super) async fn create_and_sync_workspace_attachment(
    state: &Arc<AppState>,
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
    crate::daemon::workspaces::attachments::upsert_workspace_attachment(
        state.as_ref(),
        ctx.workspace_id,
        cfg,
    )
    .await
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        )
    })?;

    crate::daemon::workspaces::attachments::sync_workspace_attachments(
        Arc::clone(state),
        &ctx.workspace,
        true,
    )
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
    state: &Arc<AppState>,
    ctx: &WorkspaceRequestContext,
    req: DeleteWorkspaceAttachmentReq,
) -> WorkspaceApiResult<Vec<WorkspaceAttachment>> {
    let removed = crate::daemon::workspaces::attachments::delete_workspace_attachment(
        state.as_ref(),
        ctx.workspace_id,
        req.kind,
        &req.name,
    )
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

    crate::daemon::workspaces::attachments::sync_workspace_attachments(
        Arc::clone(state),
        &ctx.workspace,
        false,
    )
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
