use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{SandboxBinding, Workspace, Worktree};
use ctx_settings_model::ExecutionSettings;
use ctx_store::Store;

use crate::daemon::DaemonState;

use super::retry_global_index_write;

pub async fn persist_provisioned_worktree(
    state: &Arc<DaemonState>,
    store: &Store,
    workspace: &Workspace,
    worktree: Worktree,
    sandbox_binding: Option<SandboxBinding>,
) -> anyhow::Result<Worktree> {
    store.insert_worktree(worktree.clone()).await?;
    if let Some(binding) = sandbox_binding {
        store.upsert_sandbox_binding(binding).await?;
    }
    retry_global_index_write(|| async {
        state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await
    })
    .await?;

    if let Err(err) = super::worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(state),
        workspace.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "worktree bootstrap failed: {err:?}");
    }
    if let Err(err) = crate::daemon::workspaces::attachments::sync_workspace_attachments(
        Arc::clone(state),
        workspace,
        false,
    )
    .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "attachment sync failed: {err:?}");
    }
    if let Err(err) =
        crate::daemon::workspaces::attachments::ensure_worktree_attachment_mounts_if_materialized(
            state, workspace, &worktree,
        )
        .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "attachment mounts failed: {err:?}");
    }

    Ok(worktree)
}

pub async fn provision_worktree_for_execution(
    state: &Arc<DaemonState>,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    base_commit_sha: &str,
    branch_name: &str,
    effective: &ExecutionSettings,
) -> anyhow::Result<(PathBuf, Option<SandboxBinding>)> {
    let canonical_root = ctx_workspace_services::worktree_vcs::create_managed_worktree(
        &state.core.data_root,
        &workspace.root_path,
        workspace.id,
        worktree_id,
        base_commit_sha,
        branch_name,
    )
    .await?;

    let created_at = Utc::now();
    let worktree = ctx_workspace_services::worktree_vcs::managed_worktree_record(
        workspace.id,
        worktree_id,
        &canonical_root,
        base_commit_sha,
        branch_name,
        created_at,
    );
    let binding = super::sandbox_binding::materialize_sandbox_binding_for_worktree(
        state,
        workspace,
        &worktree,
        &canonical_root,
        effective,
        created_at,
    )
    .await?;

    Ok((canonical_root, binding))
}
