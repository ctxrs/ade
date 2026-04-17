use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use ctx_core::ids::{MergeQueueEntryId, SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{MergeQueueEntry, SessionEventType, Workspace};
use ctx_store::Store;

use ctx_merge_queue::{MergeQueueHost, MergeQueueNotice, MergeQueueToolExecEvent};
pub use ctx_merge_queue::{MergeQueueSubmitParams, WorkspaceDrainStop};

use crate::daemon::AppState;
use crate::ops_events::OpsEvent;

#[async_trait]
impl MergeQueueHost for AppState {
    fn merge_queue_runtime(state: &Self) -> &ctx_merge_queue::MergeQueueRuntime {
        &state.transport.merge_queue
    }

    async fn protected_workspace_store(state: &Self, workspace_id: WorkspaceId) -> Result<Store> {
        state.store_for_workspace(workspace_id).await
    }

    async fn raw_workspace_store(state: &Self, workspace_id: WorkspaceId) -> Result<Store> {
        state.core.stores.workspace(workspace_id).await
    }

    async fn session_store(state: &Self, session_id: SessionId) -> Result<Store> {
        state.store_for_session(session_id).await
    }

    async fn worktree_store(state: &Self, worktree_id: WorktreeId) -> Result<Store> {
        state.store_for_worktree(worktree_id).await
    }

    async fn get_workspace(state: &Self, workspace_id: WorkspaceId) -> Result<Option<Workspace>> {
        state.global_store().get_workspace(workspace_id).await
    }

    async fn upsert_workspace_worktree_index(
        state: &Self,
        worktree_id: WorktreeId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        state
            .global_store()
            .upsert_workspace_worktree_index(worktree_id, workspace_id)
            .await
    }

    async fn publish_notice(state: &Arc<Self>, notice: MergeQueueNotice) -> Result<()> {
        let (session_id, payload) = match notice {
            MergeQueueNotice::Sync {
                session_id,
                worktree_id,
                target_branch,
                previous_commit_sha,
                commit_sha,
                message,
            } => (
                session_id,
                serde_json::json!({
                    "kind": "merge_queue_sync",
                    "message": message,
                    "worktree_id": worktree_id.0.to_string(),
                    "target_branch": target_branch,
                    "previous_commit_sha": previous_commit_sha,
                    "commit_sha": commit_sha,
                    "base_revision": commit_sha,
                    "base_commit_sha": commit_sha,
                }),
            ),
            MergeQueueNotice::CanonicalSync {
                session_id,
                worktree_id,
                target_branch,
                commit_sha,
                status,
                message,
            } => (
                session_id,
                serde_json::json!({
                    "kind": "merge_queue_canonical_sync",
                    "status": status,
                    "message": message,
                    "worktree_id": worktree_id.map(|id| id.0.to_string()),
                    "target_branch": target_branch,
                    "commit_sha": commit_sha,
                }),
            ),
        };
        let store = state.store_for_session(session_id).await?;
        let notice = store
            .append_session_event(session_id, None, None, SessionEventType::Notice, payload)
            .await?;
        state.publish_event(notice).await;
        Ok(())
    }

    fn emit_tool_exec(state: &Self, event: MergeQueueToolExecEvent) {
        let mut ops_event = OpsEvent::new("info", "merge_queue_tool_exec");
        ops_event.session_id = event.session_id.map(|id| id.0.to_string());
        ops_event.worktree_id = event.worktree_id.map(|id| id.0.to_string());
        ops_event.tool_kind = Some("merge_queue".to_string());
        ops_event.cwd = event.workdir.clone();
        ops_event.worktree_root = event.workdir;
        ops_event.meta = Some(serde_json::json!({
            "entry_id": event.entry_id.0.to_string(),
            "command": event.command,
            "tool_slice": event.used_tool_slice,
            "slice": event.tool_slice_unit,
        }));
        state.telemetry.ops_events.emit(ops_event);
    }
}

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
