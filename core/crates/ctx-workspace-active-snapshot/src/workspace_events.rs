use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    WorkspaceActiveSnapshotEvent, WorkspaceTaskSummary, WorktreeBootstrapNotice,
    WorktreeVcsSnapshot,
};

use crate::entry::WorkspaceActiveSnapshotEntry;
use crate::trim::compact_worktree_vcs_snapshot;
use crate::WorkspaceActiveSnapshotHub;

impl WorkspaceActiveSnapshotHub {
    pub async fn publish_session_gap(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        after_seq: i64,
        reason: Option<String>,
    ) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionGap {
            workspace_id,
            snapshot_rev,
            session_id,
            after_seq,
            reason,
        });
    }

    pub async fn publish_worktree_bootstrap(
        &self,
        workspace_id: WorkspaceId,
        notice: WorktreeBootstrapNotice,
    ) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::WorktreeBootstrap {
            workspace_id,
            snapshot_rev,
            notice,
        });
    }

    pub async fn publish_worktree_vcs_snapshot(
        &self,
        workspace_id: WorkspaceId,
        snapshot: WorktreeVcsSnapshot,
    ) {
        let snapshot = compact_worktree_vcs_snapshot(&snapshot);
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            entry
                .worktree_vcs_snapshots
                .insert(snapshot.worktree_id, snapshot.clone());
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot {
            workspace_id,
            snapshot_rev,
            snapshot: Box::new(snapshot),
        });
    }

    pub async fn hydrate_worktree_vcs_snapshots(
        &self,
        workspace_id: WorkspaceId,
        snapshots: Vec<WorktreeVcsSnapshot>,
    ) {
        if snapshots.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        for snapshot in snapshots {
            entry
                .worktree_vcs_snapshots
                .entry(snapshot.worktree_id)
                .or_insert(snapshot);
        }
    }

    pub async fn drop_worktree_vcs_snapshots(&self, worktree_ids: &[WorktreeId]) {
        if worktree_ids.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().await;
        for entry in guard.values_mut() {
            for worktree_id in worktree_ids {
                entry.worktree_vcs_snapshots.remove(worktree_id);
            }
        }
    }

    pub async fn publish_archived_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        task: WorkspaceTaskSummary,
    ) {
        let (tx, archived_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.archived_rev += 1;
            (entry.tx.clone(), entry.archived_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert {
            workspace_id,
            archived_rev,
            task: Box::new(task),
        });
    }

    pub async fn publish_archived_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        let (tx, archived_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.archived_rev += 1;
            (entry.tx.clone(), entry.archived_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ArchivedTaskDelete {
            workspace_id,
            archived_rev,
            task_id,
        });
    }
}
