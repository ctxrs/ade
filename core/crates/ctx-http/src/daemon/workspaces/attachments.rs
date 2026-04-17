use std::sync::Arc;

use std::sync::atomic::Ordering;

use anyhow::Result;
use chrono::Utc;
use ctx_core::ids::{WorkspaceAttachmentId, WorkspaceId};
use ctx_core::models::{
    Workspace, WorkspaceAttachment, WorkspaceAttachmentKind, WorkspaceAttachmentStatus, Worktree,
    WorktreeAttachmentMount, WorktreeAttachmentStatus,
};
use ctx_workspace_services::workspace_attachments;

use crate::daemon::{AppState, AttachmentMaterializationTask};
use crate::execution_effective;
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

pub(crate) async fn spawn_attachment_materialization(
    state: Arc<AppState>,
    workspace: Workspace,
    attachment_id: WorkspaceAttachmentId,
    refresh: bool,
) {
    cancel_attachment_materialization(state.as_ref(), attachment_id).await;
    let generation = state
        .workspaces
        .attachment_materialization_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let task_state = Arc::clone(&state);
    let handle = tokio::spawn(async move {
        run_attachment_materialization(Arc::clone(&task_state), workspace, attachment_id, refresh)
            .await;
        clear_attachment_materialization_task(task_state.as_ref(), attachment_id, generation).await;
    });
    let mut tasks = state.workspaces.attachment_materializations.lock().await;
    tasks.insert(
        attachment_id,
        AttachmentMaterializationTask { generation, handle },
    );
}

pub(crate) async fn cancel_attachment_materialization(
    state: &AppState,
    attachment_id: WorkspaceAttachmentId,
) {
    let existing = {
        let mut tasks = state.workspaces.attachment_materializations.lock().await;
        tasks.remove(&attachment_id)
    };
    if let Some(task) = existing {
        task.handle.abort();
        let _ = task.handle.await;
    }
}

async fn clear_attachment_materialization_task(
    state: &AppState,
    attachment_id: WorkspaceAttachmentId,
    generation: u64,
) {
    let mut tasks = state.workspaces.attachment_materializations.lock().await;
    if tasks
        .get(&attachment_id)
        .is_some_and(|task| task.generation == generation)
    {
        tasks.remove(&attachment_id);
    }
}

async fn run_attachment_materialization(
    state: Arc<AppState>,
    workspace: Workspace,
    attachment_id: WorkspaceAttachmentId,
    refresh: bool,
) {
    if let Err(err) = ctx_workspace_services::workspace_attachments::run_attachment_materialization(
        state.as_ref(),
        &workspace,
        attachment_id,
        refresh,
    )
    .await
    {
        tracing::warn!("attachment sync failed: {err:#}");
    }
}

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
    cfg: crate::attachments::AttachmentConfig,
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

#[async_trait::async_trait]
impl workspace_attachments::WorkspaceAttachmentsHost for AppState {
    fn data_root(&self) -> &std::path::Path {
        &self.core.data_root
    }

    async fn list_workspace_attachments(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceAttachment>> {
        let store = self.store_for_workspace(workspace_id).await?;
        store.list_workspace_attachments(workspace_id).await
    }

    async fn get_workspace_attachment(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<Option<WorkspaceAttachment>> {
        let store = self.store_for_workspace(workspace_id).await?;
        store.get_workspace_attachment(attachment_id).await
    }

    async fn upsert_workspace_attachment(&self, attachment: &WorkspaceAttachment) -> Result<()> {
        let store = self.store_for_workspace(attachment.workspace_id).await?;
        store.upsert_workspace_attachment(attachment).await
    }

    async fn update_workspace_attachment_status(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
        status: WorkspaceAttachmentStatus,
        last_sync_at: Option<chrono::DateTime<Utc>>,
        error_message: Option<String>,
        updated_at: chrono::DateTime<Utc>,
    ) -> Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        store
            .update_workspace_attachment_status(
                attachment_id,
                status,
                last_sync_at,
                error_message,
                updated_at,
            )
            .await
    }

    async fn delete_workspace_attachment_record(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        store.delete_workspace_attachment(attachment_id).await
    }

    async fn attachment_became_ready(
        &self,
        workspace: &Workspace,
        attachment: &WorkspaceAttachment,
    ) -> Result<()> {
        ensure_workspace_attachments_for_worktrees_with_attachments(
            self,
            workspace,
            std::slice::from_ref(attachment),
            false,
            false,
        )
        .await
    }

    async fn cleanup_removed_attachment(&self, attachment: &WorkspaceAttachment) -> Result<()> {
        crate::attachments::cleanup_removed_attachment_mounts(self, attachment).await
    }
}

pub async fn ensure_worktree_attachment_mounts(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    refresh: bool,
) -> Result<Vec<WorktreeAttachmentMount>> {
    let store = state.store_for_workspace(workspace.id).await?;
    let attachments = store.list_workspace_attachments(workspace.id).await?;
    ensure_worktree_attachment_mounts_for_attachments(
        state,
        workspace,
        worktree,
        &attachments,
        refresh,
        true,
    )
    .await
}

pub async fn ensure_worktree_attachment_mounts_if_materialized(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<Vec<WorktreeAttachmentMount>> {
    let store = state.store_for_workspace(workspace.id).await?;
    let attachments = store.list_workspace_attachments(workspace.id).await?;
    let ready = attachments
        .into_iter()
        .filter(|attachment| attachment.status == WorkspaceAttachmentStatus::Ready)
        .filter(|attachment| {
            crate::attachments::materialized_path_for_attachment(state, attachment).exists()
        })
        .collect::<Vec<_>>();
    ensure_worktree_attachment_mounts_for_attachments(
        state, workspace, worktree, &ready, false, false,
    )
    .await
}

pub async fn ensure_worktree_attachment_mounts_for_attachments(
    state: &AppState,
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
    crate::attachments::ensure_git_exclude(state, workspace, worktree.id, &worktree_root).await?;

    let mut mounts = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        match crate::attachments::ensure_attachment_mount(
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
                    materialized_id: crate::attachments::revision_key(attachment),
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

pub async fn ensure_workspace_attachments_for_worktrees(
    state: &AppState,
    workspace: &Workspace,
    refresh: bool,
) -> Result<()> {
    let store = state.store_for_workspace(workspace.id).await?;
    let attachments = store.list_workspace_attachments(workspace.id).await?;
    ensure_workspace_attachments_for_worktrees_with_attachments(
        state,
        workspace,
        &attachments,
        refresh,
        true,
    )
    .await
}

pub async fn ensure_workspace_attachments_for_worktrees_with_attachments(
    state: &AppState,
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
