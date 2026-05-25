use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{Workspace, WorkspaceAttachment, Worktree, WorktreeAttachmentMount};

use crate::daemon::{DaemonState, ProtectedWorkspaceStoreLookup};

mod hosts;
mod materialization;
mod mounts;
mod runtime;

pub(crate) use runtime::{WorkspaceAttachmentMaterializationRuntime, WorkspaceAttachmentsRuntime};

pub(crate) fn runtime_from_state(state: &Arc<DaemonState>) -> Arc<WorkspaceAttachmentsRuntime> {
    Arc::new(WorkspaceAttachmentsRuntime::new(
        state.core.data_root.clone(),
        state.core.daemon_url.clone(),
        state.global_store().clone(),
        ProtectedWorkspaceStoreLookup::new(
            state.core.stores.clone(),
            Arc::clone(&state.sessions),
            Arc::clone(&state.transport.merge_queue),
        ),
        Arc::clone(&state.execution.harness),
        Arc::clone(&state.workspaces.attachment_materialization),
    ))
}

pub async fn sync_workspace_attachments(
    state: Arc<DaemonState>,
    workspace: &Workspace,
    refresh: bool,
) -> Result<Vec<WorkspaceAttachment>> {
    runtime_from_state(&state)
        .sync_workspace_attachments(workspace, refresh)
        .await
}

pub async fn ensure_worktree_attachment_mounts_if_materialized(
    state: &Arc<DaemonState>,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<Vec<WorktreeAttachmentMount>> {
    runtime_from_state(state)
        .ensure_worktree_attachment_mounts_if_materialized(workspace, worktree)
        .await
}
