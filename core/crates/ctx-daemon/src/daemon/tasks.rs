use std::collections::HashSet;

use anyhow::Result;
use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_core::models::{
    Session, Task, TaskDeltaKind, Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor,
    WorkspaceTaskSummary,
};
use ctx_store::Store;

use crate::daemon::handle::TasksHandle;
use crate::daemon::{workspaces, WorkspaceStoreAccessError};

mod create_session;
mod create_task;
mod lifecycle;
mod route_contract;

pub use create_session::{CreateTaskSessionInput, DefaultSessionSeed, TaskSessionCreateError};
pub use create_task::{CreateTaskInput, TaskCreateError};
pub use lifecycle::{ArchiveTaskOutcome, TaskLifecycleError};
pub use route_contract::{
    ArchiveTaskRouteResponse, CreateTaskDefaultSessionRouteRequest, CreateTaskRouteRequest,
    CreateTaskSessionRouteRequest, ExecutionEnvironmentRouteValue,
    ListWorkspaceArchivedTasksRouteParams, ListWorkspaceArchivedTasksRouteRequest,
    ListWorkspaceTasksRouteParams, SessionRouteResponse, SessionStatusRouteResponse,
    SessionSummaryRouteResponse, TaskRouteError, TaskRouteErrorKind, TaskRouteParams,
    TaskRouteResponse, TaskStatusRouteResponse, UpdateTaskTitleRouteRequest,
    WorkspaceArchivedPageRouteResponse, WorkspaceIndexCursorRouteResponse,
    WorkspaceTaskSummaryRouteResponse,
};

impl TasksHandle {
    pub async fn list_workspace_tasks(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<Task>, WorkspaceStoreAccessError> {
        let store = self.state.existing_workspace_store(workspace_id).await?;
        store
            .list_tasks(workspace_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn list_workspace_archived_page(
        &self,
        workspace_id: WorkspaceId,
        cursor: Option<WorkspaceIndexCursor>,
        limit: i64,
    ) -> Result<WorkspaceArchivedPage, WorkspaceStoreAccessError> {
        let store = self.state.existing_workspace_store(workspace_id).await?;
        let (tasks, next_cursor): (Vec<WorkspaceTaskSummary>, Option<WorkspaceIndexCursor>) = store
            .list_workspace_archived_page(workspace_id, cursor, limit)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        let (_, total_archived) = store
            .workspace_task_counts(workspace_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        let (_, archived_rev) =
            workspaces::load_workspace_active_snapshot_state(&self.state, workspace_id).await;

        Ok(WorkspaceArchivedPage {
            workspace_id,
            archived_rev,
            tasks,
            next_cursor,
            total_archived,
        })
    }

    pub async fn list_task_sessions(&self, task_id: TaskId) -> Result<Option<Vec<Session>>> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        store.list_sessions_for_task(task_id).await.map(Some)
    }

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
        let updated = if read {
            store.mark_task_read(task_id).await?
        } else {
            store.mark_task_unread(task_id).await?
        };
        if !updated {
            return Ok(None);
        }
        let task = store.get_task_with_activity(task_id).await?;
        if let Some(task) = task.as_ref() {
            self.publish_task_updated(task_id, task.clone()).await;
        }
        Ok(task)
    }

    pub async fn update_task_title(&self, task_id: TaskId, title: String) -> Result<Option<Task>> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        let updated = store.update_task_title(task_id, title).await?;
        if !updated {
            return Ok(None);
        }
        let Some(task) = store.get_task_with_activity(task_id).await? else {
            return Ok(None);
        };
        let sessions = store
            .list_sessions_for_task(task_id)
            .await
            .unwrap_or_default();
        let session_ids = sessions
            .iter()
            .map(|session| session.id.0.to_string())
            .collect();
        let mut worktree_ids = sessions
            .iter()
            .map(|session| session.worktree_id)
            .collect::<HashSet<_>>();
        if let Some(primary_worktree_id) = task.primary_worktree_id {
            worktree_ids.insert(primary_worktree_id);
        }
        let mut worktree_id_strings = HashSet::new();
        for worktree_id in worktree_ids {
            match store.get_worktree(worktree_id).await {
                Ok(Some(worktree)) => {
                    worktree_id_strings.insert(worktree.id.0.to_string());
                }
                Ok(None) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree_id.0,
                        "worktree missing for task title update"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree_id.0,
                        "failed to load worktree for task title update: {error:?}"
                    );
                }
            }
        }

        self.publish_task_updated(task_id, task.clone()).await;
        if let Err(error) = self
            .state
            .transport
            .web_sessions
            .close_for_task(&session_ids, &worktree_id_strings)
            .await
        {
            tracing::warn!(task_id = %task_id.0, "failed to close web sessions for title update: {error:?}");
        }

        Ok(Some(task))
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

    async fn task_store_or_none(&self, task_id: TaskId) -> Result<Option<ctx_store::Store>> {
        let Some(workspace_id) = self
            .state
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await?
        else {
            return Ok(None);
        };
        match self.state.existing_workspace_store(workspace_id).await {
            Ok(store) => Ok(Some(store)),
            Err(WorkspaceStoreAccessError::NotFound) => Ok(None),
            Err(WorkspaceStoreAccessError::Unavailable(error)) => Err(error),
        }
    }

    async fn load_task_context(
        &self,
        task_id: TaskId,
    ) -> Result<Option<(Store, Task, Workspace)>, TaskLifecycleError> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        let Some(task) = store
            .get_task(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?
        else {
            return Ok(None);
        };
        let workspace = self
            .state
            .global_store()
            .get_workspace(task.workspace_id)
            .await
            .map_err(TaskLifecycleError::Internal)?;
        let Some(workspace) = workspace else {
            return Ok(None);
        };
        Ok(Some((store, task, workspace)))
    }
}
