use std::sync::Arc;

use anyhow::Result;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, WorkspaceAttachment, WorkspaceAttachmentKind};
use ctx_workspace_services::workspace_attachments;

use crate::daemon::AppState;

mod hosts;
mod materialization;
mod mounts;

pub use mounts::{
    ensure_workspace_attachments_for_worktrees_with_attachments,
    ensure_worktree_attachment_mounts_if_materialized,
};

use self::materialization::{cancel_attachment_materialization, spawn_attachment_materialization};

pub async fn sync_workspace_attachments(
    state: Arc<AppState>,
    workspace: &Workspace,
    refresh: bool,
) -> Result<Vec<WorkspaceAttachment>> {
    let result =
        workspace_attachments::sync_workspace_attachments(state.as_ref(), workspace, refresh)
            .await?;
    for plan in result.plans {
        spawn_attachment_materialization(
            Arc::clone(&state),
            workspace.clone(),
            plan.id,
            plan.refresh,
        )
        .await;
    }
    Ok(result.attachments)
}

pub async fn upsert_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    cfg: ctx_workspace_attachments::AttachmentConfig,
) -> Result<WorkspaceAttachment> {
    workspace_attachments::upsert_workspace_attachment(state, workspace_id, cfg).await
}

pub async fn delete_workspace_attachment(
    state: &AppState,
    workspace_id: WorkspaceId,
    kind: WorkspaceAttachmentKind,
    name: &str,
) -> Result<bool> {
    let Some(target) =
        workspace_attachments::find_workspace_attachment(state, workspace_id, kind, name).await?
    else {
        return Ok(false);
    };
    cancel_attachment_materialization(state, target.id).await;
    workspace_attachments::delete_workspace_attachment(state, &target).await?;
    Ok(true)
}
