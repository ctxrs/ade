use std::sync::atomic::Ordering;
use std::sync::Arc;

use ctx_core::ids::WorkspaceAttachmentId;
use ctx_core::models::Workspace;

use crate::daemon::{AppState, AttachmentMaterializationTask};

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
