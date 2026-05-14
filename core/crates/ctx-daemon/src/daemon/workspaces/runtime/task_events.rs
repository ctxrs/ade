use anyhow::Result;
use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_core::models::Task;

use crate::daemon::state::{DaemonState, WorkspaceRuntime};

impl WorkspaceRuntime {
    pub async fn emit_workspace_task_upsert(
        &self,
        state: &DaemonState,
        task_id: TaskId,
    ) -> Result<()> {
        let mut task: Option<Task> = None;
        let store = state.store_for_task(task_id).await?;
        match store.get_workspace_active_task_summary(task_id).await? {
            Some(summary) => {
                let workspace_id = summary.task.workspace_id;
                task = Some(summary.task.clone());
                self.workspace_active_snapshot
                    .publish_active_task_upsert(workspace_id, summary)
                    .await;
            }
            None => {
                if let Some(loaded) = store.get_task(task_id).await? {
                    task = Some(loaded.clone());
                    self.workspace_active_snapshot
                        .publish_active_task_delete(loaded.workspace_id, task_id)
                        .await;
                }
            }
        }

        if let Some(task) = task.as_ref().filter(|task| task.archived_at.is_some()) {
            let _ = self.emit_workspace_archived_task_upsert(state, task).await;
        }
        Ok(())
    }

    pub async fn emit_workspace_task_delete(
        &self,
        state: &DaemonState,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        if let Err(err) = state.store_for_workspace(workspace_id).await {
            tracing::warn!(
                workspace_id = %workspace_id.0,
                task_id = %task_id.0,
                "workspace task delete store missing: {err:#}"
            );
            return;
        }
        self.workspace_active_snapshot
            .publish_active_task_delete(workspace_id, task_id)
            .await;
    }

    pub async fn emit_workspace_archived_task_delete(
        &self,
        state: &DaemonState,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        let updated = match state.store_for_workspace(workspace_id).await {
            Ok(store) => match store
                .bump_workspace_archived_snapshot_rev(workspace_id)
                .await
            {
                Ok(_) => true,
                Err(err) => {
                    tracing::warn!(
                        workspace_id = %workspace_id.0,
                        task_id = %task_id.0,
                        "workspace archived delete read model update failed: {err:#}"
                    );
                    false
                }
            },
            Err(err) => {
                tracing::warn!(
                    workspace_id = %workspace_id.0,
                    task_id = %task_id.0,
                    "workspace archived delete read model store missing: {err:#}"
                );
                false
            }
        };
        if updated {
            self.workspace_active_snapshot
                .publish_archived_task_delete(workspace_id, task_id)
                .await;
        }
    }

    async fn emit_workspace_archived_task_upsert(
        &self,
        state: &DaemonState,
        task: &Task,
    ) -> Result<()> {
        let store = state.store_for_task(task.id).await?;
        let Some(summary) = store.get_workspace_task_summary(task.id).await? else {
            return Ok(());
        };
        if summary.task.archived_at.is_none() {
            return Ok(());
        }

        let _ = store
            .bump_workspace_archived_snapshot_rev(task.workspace_id)
            .await?;
        self.workspace_active_snapshot
            .publish_archived_task_upsert(task.workspace_id, summary)
            .await;
        Ok(())
    }
}
