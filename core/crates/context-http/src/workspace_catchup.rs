use std::collections::HashMap;

use tokio::sync::{broadcast, Mutex};

use context_core::ids::{TaskId, WorkspaceId};
use context_core::models::{
    SessionCatchupSummary, SessionHeadDelta, WorkspaceCatchupEvent, WorkspaceCatchupTaskSummary,
    WorkspaceCatchupTrackSummary, WorktreeBootstrapNotice,
};

pub struct WorkspaceCatchupHub {
    inner: Mutex<HashMap<WorkspaceId, WorkspaceCatchupEntry>>,
}

struct WorkspaceCatchupEntry {
    rev: i64,
    tx: broadcast::Sender<WorkspaceCatchupEvent>,
}

impl WorkspaceCatchupEntry {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self { rev: 0, tx }
    }
}

impl WorkspaceCatchupHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    async fn ensure_entry(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Sender<WorkspaceCatchupEvent> {
        let mut guard = self.inner.lock().await;
        guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceCatchupEntry::new)
            .tx
            .clone()
    }

    pub async fn current_rev(&self, workspace_id: WorkspaceId) -> i64 {
        let guard = self.inner.lock().await;
        guard.get(&workspace_id).map(|entry| entry.rev).unwrap_or(0)
    }

    pub async fn subscribe(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Receiver<WorkspaceCatchupEvent> {
        self.ensure_entry(workspace_id).await.subscribe()
    }

    pub async fn publish_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        task: WorkspaceCatchupTaskSummary,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceCatchupEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceCatchupEvent::TaskUpsert {
            workspace_id,
            snapshot_rev: entry.rev,
            task,
        });
    }

    pub async fn publish_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceCatchupEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceCatchupEvent::TaskDelete {
            workspace_id,
            snapshot_rev: entry.rev,
            task_id,
        });
    }

    pub async fn publish_track_upsert(
        &self,
        workspace_id: WorkspaceId,
        track: WorkspaceCatchupTrackSummary,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceCatchupEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceCatchupEvent::TrackUpsert {
            workspace_id,
            snapshot_rev: entry.rev,
            track,
        });
    }

    pub async fn publish_session_summary(
        &self,
        workspace_id: WorkspaceId,
        summary: SessionCatchupSummary,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceCatchupEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceCatchupEvent::SessionSummary {
            workspace_id,
            snapshot_rev: entry.rev,
            summary,
        });
    }

    pub async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        delta: SessionHeadDelta,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceCatchupEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceCatchupEvent::SessionHeadDelta {
            workspace_id,
            snapshot_rev: entry.rev,
            delta: Box::new(delta),
        });
    }

    pub async fn publish_worktree_bootstrap(
        &self,
        workspace_id: WorkspaceId,
        notice: WorktreeBootstrapNotice,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceCatchupEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceCatchupEvent::WorktreeBootstrap {
            workspace_id,
            snapshot_rev: entry.rev,
            notice,
        });
    }
}

impl Default for WorkspaceCatchupHub {
    fn default() -> Self {
        Self::new()
    }
}
