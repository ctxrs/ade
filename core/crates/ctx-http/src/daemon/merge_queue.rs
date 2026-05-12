use std::sync::Arc;

use anyhow::Result;

use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_core::models::MergeQueueEntry;

use ctx_merge_queue::MergeQueueSubmitParams;
#[cfg(test)]
use ctx_merge_queue::WorkspaceDrainStop;

use crate::daemon::AppState;

mod host;

pub async fn get_workspace_merge_queue_entry(
    state: &AppState,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::get_workspace_merge_queue_entry::<AppState>(state, workspace_id, entry_id)
        .await
}

pub async fn submit_merge_queue_entry(
    state: &Arc<AppState>,
    params: MergeQueueSubmitParams,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::submit_merge_queue_entry::<AppState>(state, params).await
}

pub async fn cancel_merge_queue_entry(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::cancel_merge_queue_entry::<AppState>(state, workspace_id, entry_id).await
}

pub async fn retry_merge_queue_entry(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::retry_merge_queue_entry::<AppState>(state, workspace_id, entry_id).await
}

pub fn spawn_merge_queue_runner(state: Arc<AppState>) {
    ctx_merge_queue::spawn_merge_queue_runner::<AppState>(state);
}

pub(crate) async fn schedule_workspace_if_enabled_and_queued(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<bool> {
    ctx_merge_queue::schedule_workspace_if_enabled_and_queued::<AppState>(state, workspace_id).await
}

pub(crate) async fn activate_workspace_merge_queue(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) {
    ctx_merge_queue::activate_workspace_merge_queue::<AppState>(state, workspace_id).await;
}

pub(crate) async fn cancel_queued_entries_for_disabled_workspace(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace_id: WorkspaceId,
) -> Result<()> {
    ctx_merge_queue::cancel_queued_entries_for_disabled_workspace::<AppState>(
        state,
        store,
        workspace_id,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn list_queued_entries_for_workspace(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> Result<Vec<MergeQueueEntry>> {
    ctx_merge_queue::list_queued_entries_for_workspace::<AppState>(state, workspace_id).await
}

#[cfg(test)]
pub(crate) async fn begin_workspace_drain(state: &AppState, workspace_id: WorkspaceId) -> bool {
    ctx_merge_queue::begin_workspace_drain::<AppState>(state, workspace_id).await
}

#[cfg(test)]
pub(crate) async fn finish_workspace_drain(state: &AppState, workspace_id: WorkspaceId) -> bool {
    ctx_merge_queue::finish_workspace_drain::<AppState>(state, workspace_id).await
}

#[cfg(test)]
pub(crate) async fn schedule_workspace_drain(state: &Arc<AppState>, workspace_id: WorkspaceId) {
    ctx_merge_queue::schedule_workspace_drain::<AppState>(state, workspace_id).await;
}

#[cfg(test)]
pub(crate) async fn reschedule_workspace_after_drain(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    stop: WorkspaceDrainStop,
) -> bool {
    ctx_merge_queue::reschedule_workspace_after_drain::<AppState>(state, workspace_id, stop).await
}

#[cfg(test)]
mod tests;
