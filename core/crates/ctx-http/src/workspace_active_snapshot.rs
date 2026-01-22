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
    tx: broadcast::Sender<WorkspaceActiveSnapshotEvent>,
}

impl WorkspaceActiveSnapshotEntry {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self { tx }
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

    pub async fn subscribe(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Receiver<WorkspaceActiveSnapshotEvent> {
        self.ensure_entry(workspace_id).await.subscribe()
    }

    pub async fn publish_active_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task: WorkspaceActiveTaskSummary,
    ) {
        let tx = self.ensure_entry(workspace_id).await;
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
            workspace_id,
            snapshot_rev,
            task: Box::new(task),
        });
    }

    pub async fn publish_active_task_delete(
        &self,
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task_id: TaskId,
    ) {
        let tx = self.ensure_entry(workspace_id).await;
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ActiveTaskDelete {
            workspace_id,
            snapshot_rev,
            task_id,
        });
    }

    pub async fn publish_session_summary(
        &self,
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        summary: SessionSnapshotSummary,
    ) {
        let tx = self.ensure_entry(workspace_id).await;
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionSummary {
            workspace_id,
            snapshot_rev,
            summary: Box::new(summary),
        });
    }

    pub async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        delta: SessionHeadDelta,
    ) {
        let tx = self.ensure_entry(workspace_id).await;
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            workspace_id,
            snapshot_rev,
            delta: Box::new(delta),
        });
    }

    pub async fn publish_worktree_bootstrap(
        &self,
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        notice: WorktreeBootstrapNotice,
    ) {
        let tx = self.ensure_entry(workspace_id).await;
        let _ = tx.send(WorkspaceActiveSnapshotEvent::WorktreeBootstrap {
            workspace_id,
            snapshot_rev,
            notice,
        });
    }

    pub async fn publish_archived_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        archived_rev: i64,
        task: WorkspaceTaskSummary,
        snapshot: Option<SessionSnapshot>,
    ) {
        let tx = self.ensure_entry(workspace_id).await;
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert {
            workspace_id,
            archived_rev,
            task: Box::new(task),
            snapshot: snapshot.map(Box::new),
        });
    }

    pub async fn publish_archived_task_delete(
        &self,
        workspace_id: WorkspaceId,
        archived_rev: i64,
        task_id: TaskId,
    ) {
        let tx = self.ensure_entry(workspace_id).await;
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ArchivedTaskDelete {
            workspace_id,
            archived_rev,
            task_id,
        });
    }
}

impl Default for WorkspaceActiveSnapshotHub {
    fn default() -> Self {
        Self::new()
    }
}
