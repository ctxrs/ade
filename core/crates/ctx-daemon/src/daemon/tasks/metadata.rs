use anyhow::Result;
use ctx_core::ids::TaskId;
use ctx_core::models::{Task, TaskDeltaKind};

use crate::daemon::handle::TasksHandle;

impl TasksHandle {
    pub async fn mark_task_read(&self, task_id: TaskId) -> Result<Option<Task>> {
        self.set_task_read_state(task_id, true).await
    }

    pub async fn mark_task_unread(&self, task_id: TaskId) -> Result<Option<Task>> {
        self.set_task_read_state(task_id, false).await
    }

    async fn set_task_read_state(&self, task_id: TaskId, read: bool) -> Result<Option<Task>> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        let task = ctx_task_service::metadata::set_task_read_state(&store, task_id, read).await?;
        if let Some(task) = task.as_ref() {
            self.publish_task_updated(task_id, task.clone()).await;
        }
        Ok(task)
    }

    pub async fn update_task_title(&self, task_id: TaskId, title: String) -> Result<Option<Task>> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        let Some(outcome) =
            ctx_task_service::metadata::update_task_title_record(&store, task_id, title).await?
        else {
            return Ok(None);
        };
        let session_ids = outcome
            .session_ids
            .iter()
            .map(|session_id| session_id.0.to_string())
            .collect();
        let worktree_id_strings = outcome
            .worktree_ids
            .iter()
            .map(|worktree_id| worktree_id.0.to_string())
            .collect();

        self.publish_task_updated(task_id, outcome.task.clone())
            .await;
        if let Err(error) = self
            .state
            .transport
            .web_sessions
            .close_for_task(&session_ids, &worktree_id_strings)
            .await
        {
            tracing::warn!(task_id = %task_id.0, "failed to close web sessions for title update: {error:?}");
        }

        Ok(Some(outcome.task))
    }

    async fn publish_task_updated(&self, task_id: TaskId, task: Task) {
        let _ = self
            .state
            .emit_workspace_task_delta(task, TaskDeltaKind::Updated)
            .await;
        if let Err(error) = self.state.emit_workspace_task_upsert(task_id).await {
            tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {error:?}");
        }
    }
}
