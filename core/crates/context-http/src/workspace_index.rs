use std::collections::HashMap;

use tokio::sync::{broadcast, Mutex};

use context_core::ids::{TaskId, WorkspaceId};
use context_core::models::{WorkspaceIndexEvent, WorkspaceTaskSummary};

pub struct WorkspaceIndexHub {
    inner: Mutex<HashMap<WorkspaceId, WorkspaceIndexEntry>>,
}

struct WorkspaceIndexEntry {
    rev: i64,
    tx: broadcast::Sender<WorkspaceIndexEvent>,
}

impl WorkspaceIndexEntry {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self { rev: 0, tx }
    }
}

impl WorkspaceIndexHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    async fn ensure_entry(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Sender<WorkspaceIndexEvent> {
        let mut guard = self.inner.lock().await;
        guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceIndexEntry::new)
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
    ) -> broadcast::Receiver<WorkspaceIndexEvent> {
        self.ensure_entry(workspace_id).await.subscribe()
    }

    pub async fn publish_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        summary: WorkspaceTaskSummary,
    ) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceIndexEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceIndexEvent::TaskUpsert {
            workspace_id,
            snapshot_rev: entry.rev,
            task: summary,
        });
    }

    pub async fn publish_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceIndexEntry::new);
        entry.rev += 1;
        let _ = entry.tx.send(WorkspaceIndexEvent::TaskDelete {
            workspace_id,
            snapshot_rev: entry.rev,
            task_id,
        });
    }
}
