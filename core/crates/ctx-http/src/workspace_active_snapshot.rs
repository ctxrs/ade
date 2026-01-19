use std::collections::HashMap;

use tokio::sync::{broadcast, Mutex};

use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_core::models::{
    SessionHeadDelta, SessionSnapshot, SessionSnapshotSummary, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveTaskSummary, WorkspaceTaskSummary, WorktreeBootstrapNotice,
};

pub struct WorkspaceActiveSnapshotHub {
    inner: Mutex<HashMap<WorkspaceId, WorkspaceActiveSnapshotEntry>>,
}

struct WorkspaceActiveSnapshotEntry {
    rev: i64,
    archived_rev: i64,
    tx: broadcast::Sender<WorkspaceActiveSnapshotEvent>,
}

impl WorkspaceActiveSnapshotEntry {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self {
            rev: 0,
            archived_rev: 0,
            tx,
        }
    }
}

impl WorkspaceActiveSnapshotHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    async fn ensure_entry(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Sender<WorkspaceActiveSnapshotEvent> {
        let mut guard = self.inner.lock().await;
        guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new)
            .tx
            .clone()
    }

    pub async fn current_rev(&self, workspace_id: WorkspaceId) -> i64 {
        let guard = self.inner.lock().await;
        guard.get(&workspace_id).map(|entry| entry.rev).unwrap_or(0)
    }

    pub async fn current_archived_rev(&self, workspace_id: WorkspaceId) -> i64 {
        let guard = self.inner.lock().await;
        guard
            .get(&workspace_id)
            .map(|entry| entry.archived_rev)
            .unwrap_or(0)
    }

    pub async fn subscribe(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Receiver<WorkspaceActiveSnapshotEvent> {
        self.ensure_entry(workspace_id).await.subscribe()
    }

    pub async fn publish_active_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        task: WorkspaceActiveTaskSummary,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.rev += 1;
        let _ = entry
            .tx
            .send(WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
                workspace_id,
                snapshot_rev: entry.rev,
                task: Box::new(task),
            });
    }

    pub async fn publish_active_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.rev += 1;
        let _ = entry
            .tx
            .send(WorkspaceActiveSnapshotEvent::ActiveTaskDelete {
                workspace_id,
                snapshot_rev: entry.rev,
                task_id,
            });
    }

    pub async fn publish_session_summary(
        &self,
        workspace_id: WorkspaceId,
        summary: SessionSnapshotSummary,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceActiveSnapshotEvent::SessionSummary {
            workspace_id,
            snapshot_rev: entry.rev,
            summary: Box::new(summary),
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
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.rev += 1;
        let _ = entry
            .tx
            .send(WorkspaceActiveSnapshotEvent::SessionHeadDelta {
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
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.rev += 1;
        let _ = entry
            .tx
            .send(WorkspaceActiveSnapshotEvent::WorktreeBootstrap {
                workspace_id,
                snapshot_rev: entry.rev,
                notice,
            });
    }

    pub async fn publish_archived_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        task: WorkspaceTaskSummary,
        snapshot: Option<SessionSnapshot>,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.archived_rev += 1;
        let _ = entry
            .tx
            .send(WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert {
                workspace_id,
                archived_rev: entry.archived_rev,
                task: Box::new(task),
                snapshot: snapshot.map(Box::new),
            });
    }

    pub async fn publish_archived_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.archived_rev += 1;
        let _ = entry
            .tx
            .send(WorkspaceActiveSnapshotEvent::ArchivedTaskDelete {
                workspace_id,
                archived_rev: entry.archived_rev,
                task_id,
            });
    }
}

impl Default for WorkspaceActiveSnapshotHub {
    fn default() -> Self {
        Self::new()
    }
}
