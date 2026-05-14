use anyhow::Result;
use chrono::Utc;
use ctx_core::models::{
    Workspace, WorkspaceAttachment, WorkspaceAttachmentStatus, Worktree, WorktreeAttachmentMount,
    WorktreeAttachmentStatus,
};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use crate::daemon::execution_effective;
use crate::daemon::DaemonState;

pub async fn ensure_worktree_attachment_mounts_if_materialized(
    state: &DaemonState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<Vec<WorktreeAttachmentMount>> {
    let store = state.store_for_workspace(workspace.id).await?;
    let attachments = store.list_workspace_attachments(workspace.id).await?;
    let ready = attachments
        .into_iter()
        .filter(|attachment| attachment.status == WorkspaceAttachmentStatus::Ready)
        .filter(|attachment| {
            ctx_workspace_attachments::materialized_path_for_attachment(
                &state.core.data_root,
                attachment,
            )
            .exists()
        })
        .collect::<Vec<_>>();
    ensure_worktree_attachment_mounts_for_attachments(
        state, workspace, worktree, &ready, false, false,
    )
    .await
}

pub async fn ensure_worktree_attachment_mounts_for_attachments(
    state: &DaemonState,
    workspace: &Workspace,
    worktree: &Worktree,
    attachments: &[WorkspaceAttachment],
    refresh: bool,
    materialize: bool,
) -> Result<Vec<WorktreeAttachmentMount>> {
    if attachments.is_empty() {
        return Ok(vec![]);
    }

    let store = state.store_for_workspace(workspace.id).await?;
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let _effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    let worktree_root = data_plane.live_worktree_root;
    ctx_workspace_attachments::ensure_git_exclude(state, workspace, worktree.id, &worktree_root)
        .await?;

    let mut mounts = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        match ctx_workspace_attachments::ensure_attachment_mount(
            state,
            workspace,
            worktree.id,
            &worktree_root,
            attachment,
            refresh,
            materialize,
        )
        .await
        {
            Ok(mount) => mounts.push(mount),
            Err(err) => {
                let now = Utc::now();
                let mount = WorktreeAttachmentMount {
                    worktree_id: worktree.id,
                    attachment_id: attachment.id,
                    mount_abs_path: worktree_root
                        .join(&attachment.mount_relpath)
                        .to_string_lossy()
                        .to_string(),
                    materialized_id: ctx_workspace_attachments::revision_key(attachment),
                    status: WorktreeAttachmentStatus::Error,
                    last_sync_at: Some(now),
                    error_message: Some(err.to_string()),
                    created_at: now,
                    updated_at: now,
                };
                store.upsert_worktree_attachment_mount(&mount).await?;
                mounts.push(mount);
            }
        }
    }

    Ok(mounts)
}

pub async fn ensure_workspace_attachments_for_worktrees_with_attachments(
    state: &DaemonState,
    workspace: &Workspace,
    attachments: &[WorkspaceAttachment],
    refresh: bool,
    materialize: bool,
) -> Result<()> {
    let store = state.store_for_workspace(workspace.id).await?;
    let worktrees = store.list_worktrees(workspace.id).await?;
    for worktree in worktrees {
        let _ = ensure_worktree_attachment_mounts_for_attachments(
            state,
            workspace,
            &worktree,
            attachments,
            refresh,
            materialize,
        )
        .await;
    }
    Ok(())
}
