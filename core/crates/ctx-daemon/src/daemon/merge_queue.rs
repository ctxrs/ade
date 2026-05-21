use std::sync::Arc;

use anyhow::Result;

use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_core::models::MergeQueueEntry;

use ctx_merge_queue::MergeQueueSubmitParams;
#[cfg(test)]
use ctx_merge_queue::WorkspaceDrainStop;

use crate::daemon::{DaemonState, WorkspacesHandle};

mod host;
mod route_contract;
mod submit_route;

pub use route_contract::{
    ListMergeQueueEntriesRouteRequest, MergeQueueEntryRouteError, MergeQueueEntryRouteErrorKind,
    MergeQueueEntryRouteParams, MergeQueueEntryRouteResponse, MergeQueueLogDownloadRouteError,
    MergeQueueLogDownloadRouteErrorKind,
};

pub async fn get_workspace_merge_queue_entry(
    state: &DaemonState,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::get_workspace_merge_queue_entry::<DaemonState>(state, workspace_id, entry_id)
        .await
}

pub async fn submit_merge_queue_entry(
    state: &Arc<DaemonState>,
    params: MergeQueueSubmitParams,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::submit_merge_queue_entry::<DaemonState>(state, params).await
}

pub async fn cancel_merge_queue_entry(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::cancel_merge_queue_entry::<DaemonState>(state, workspace_id, entry_id).await
}

pub async fn retry_merge_queue_entry(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    ctx_merge_queue::retry_merge_queue_entry::<DaemonState>(state, workspace_id, entry_id).await
}

pub fn spawn_merge_queue_runner(state: Arc<DaemonState>) {
    ctx_merge_queue::spawn_merge_queue_runner::<DaemonState>(state);
}

pub async fn schedule_workspace_if_enabled_and_queued(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<bool> {
    ctx_merge_queue::schedule_workspace_if_enabled_and_queued::<DaemonState>(state, workspace_id)
        .await
}

pub async fn activate_workspace_merge_queue(state: &Arc<DaemonState>, workspace_id: WorkspaceId) {
    ctx_merge_queue::activate_workspace_merge_queue::<DaemonState>(state, workspace_id).await;
}

impl WorkspacesHandle {
    pub(in crate::daemon) async fn submit_merge_queue_entry(
        &self,
        params: MergeQueueSubmitParams,
    ) -> Result<MergeQueueEntry> {
        submit_merge_queue_entry(&self.state, params).await
    }

    pub(in crate::daemon) async fn cancel_merge_queue_entry(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> Result<MergeQueueEntry> {
        cancel_merge_queue_entry(&self.state, workspace_id, entry_id).await
    }

    pub(in crate::daemon) async fn retry_merge_queue_entry(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> Result<MergeQueueEntry> {
        retry_merge_queue_entry(&self.state, workspace_id, entry_id).await
    }

    pub(in crate::daemon) async fn get_workspace_merge_queue_entry(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> Result<MergeQueueEntry> {
        get_workspace_merge_queue_entry(self.state.as_ref(), workspace_id, entry_id).await
    }
}

pub async fn cancel_queued_entries_for_disabled_workspace(
    state: &Arc<DaemonState>,
    store: &ctx_store::Store,
    workspace_id: WorkspaceId,
) -> Result<()> {
    ctx_merge_queue::cancel_queued_entries_for_disabled_workspace::<DaemonState>(
        state,
        store,
        workspace_id,
    )
    .await
}

#[cfg(test)]
pub async fn list_queued_entries_for_workspace(
    state: &DaemonState,
    workspace_id: WorkspaceId,
) -> Result<Vec<MergeQueueEntry>> {
    ctx_merge_queue::list_queued_entries_for_workspace::<DaemonState>(state, workspace_id).await
}

#[cfg(test)]
pub async fn begin_workspace_drain(state: &DaemonState, workspace_id: WorkspaceId) -> bool {
    ctx_merge_queue::begin_workspace_drain::<DaemonState>(state, workspace_id).await
}

#[cfg(test)]
pub async fn finish_workspace_drain(state: &DaemonState, workspace_id: WorkspaceId) -> bool {
    ctx_merge_queue::finish_workspace_drain::<DaemonState>(state, workspace_id).await
}

#[cfg(test)]
pub async fn schedule_workspace_drain(state: &Arc<DaemonState>, workspace_id: WorkspaceId) {
    ctx_merge_queue::schedule_workspace_drain::<DaemonState>(state, workspace_id).await;
}

#[cfg(test)]
pub async fn reschedule_workspace_after_drain(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    stop: WorkspaceDrainStop,
) -> bool {
    ctx_merge_queue::reschedule_workspace_after_drain::<DaemonState>(state, workspace_id, stop)
        .await
}

#[cfg(test)]
mod tests;
