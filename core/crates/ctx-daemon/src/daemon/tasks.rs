use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Session, Task, TaskDeltaKind, Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor,
    WorkspaceTaskSummary, Worktree,
};
use ctx_store::Store;
use ctx_workspace_services::worktree_vcs::ensure_worktree_attached;

use crate::daemon::handle::TasksHandle;
use crate::daemon::workspaces::{BranchCleanupErrorMode, TaskWorktreeCleanupTarget};
use crate::daemon::{workspaces, WorkspaceStoreAccessError};

mod create_session;
mod create_task;

pub use create_session::{CreateTaskSessionInput, DefaultSessionSeed, TaskSessionCreateError};
pub use create_task::{CreateTaskInput, TaskCreateError};

pub struct ArchiveTaskOutcome {
    pub task: Task,
    pub cleanup_failed: bool,
}

#[derive(Debug)]
pub enum TaskLifecycleError {
    NotFound,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for TaskLifecycleError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

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

    pub async fn archive_task(
        &self,
        task_id: TaskId,
    ) -> Result<ArchiveTaskOutcome, TaskLifecycleError> {
        let Some((store, task, workspace)) = self.load_task_context(task_id).await? else {
            return Err(TaskLifecycleError::NotFound);
        };
        let sessions = store
            .list_all_sessions_for_task(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?;
        let session_ids: Vec<SessionId> = sessions.iter().map(|session| session.id).collect();
        for session in &sessions {
            self.state.cleanup_session(session.id).await;
        }
        let worktrees = load_archive_worktrees(&store, &task, &sessions).await?;

        let updated = store
            .archive_task(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?;
        if !updated {
            return Err(TaskLifecycleError::NotFound);
        }

        let mut errors = Vec::new();
        let mut cleanup_targets = Vec::new();
        for worktree in &worktrees {
            let other_active = match store
                .count_active_tasks_for_worktree(worktree.id, Some(task_id))
                .await
            {
                Ok(count) => count > 0,
                Err(error) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        "failed to check worktree usage: {error:#}"
                    );
                    true
                }
            };
            if other_active {
                continue;
            }
            let sandbox_binding = match store.get_sandbox_binding(worktree.id).await {
                Ok(binding) => binding,
                Err(error) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        "failed to load sandbox binding for cleanup: {error:#}"
                    );
                    None
                }
            };
            cleanup_targets.push(TaskWorktreeCleanupTarget {
                managed_root: workspaces::managed_worktree_root(
                    self.state.as_ref(),
                    &workspace,
                    worktree,
                ),
                sandbox_binding,
                worktree: worktree.clone(),
                destroy_worktree_on_cleanup: true,
            });
        }
        errors.extend(
            workspaces::cleanup_task_worktrees(
                self.state.as_ref(),
                &workspace,
                task_id,
                &cleanup_targets,
                BranchCleanupErrorMode::Report,
            )
            .await,
        );
        let cleanup_failed = !errors.is_empty();
        if cleanup_failed {
            tracing::warn!(
                task_id = %task_id.0,
                "archive cleanup had errors after task state was persisted"
            );
        }
        let task = store
            .get_task_with_activity(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?
            .ok_or(TaskLifecycleError::NotFound)?;
        let _ = self
            .state
            .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Archived)
            .await;
        if let Err(error) = self.state.emit_workspace_task_upsert(task_id).await {
            tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {error:?}");
        }
        for session_id in session_ids {
            self.state
                .workspaces
                .workspace_active_snapshot
                .remove_session(session_id)
                .await;
        }
        Ok(ArchiveTaskOutcome {
            task,
            cleanup_failed,
        })
    }

    pub async fn unarchive_task(&self, task_id: TaskId) -> Result<Task, TaskLifecycleError> {
        let Some((store, task, workspace)) = self.load_task_context(task_id).await? else {
            return Err(TaskLifecycleError::NotFound);
        };
        let UnarchiveWorktreePlan {
            session_ids,
            managed_worktrees,
            worktrees,
        } = load_unarchive_worktree_plan(self, &store, &workspace, &task).await?;

        for (worktree, root) in &managed_worktrees {
            let branch = worktree.git_branch.as_deref().unwrap_or_default();
            ensure_worktree_attached(
                &workspace.root_path,
                root,
                &worktree.base_commit_sha,
                branch,
            )
            .await
            .map_err(|error| {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to recreate worktree: {error:#}"
                );
                TaskLifecycleError::Internal(error)
            })?;
        }

        for worktree in &worktrees {
            let sandbox_binding = store
                .get_sandbox_binding(worktree.id)
                .await
                .map_err(TaskLifecycleError::Internal)?;
            if let Some(binding) = sandbox_binding.as_ref() {
                let refreshed_binding = workspaces::rematerialize_sandbox_binding_for_worktree(
                    &self.state,
                    &workspace,
                    worktree,
                    binding,
                )
                .await
                .map_err(|error| {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        "failed to rematerialize sandbox worktree on unarchive: {error:#}"
                    );
                    TaskLifecycleError::Internal(error)
                })?;
                store
                    .upsert_sandbox_binding(refreshed_binding)
                    .await
                    .map_err(TaskLifecycleError::Internal)?;
            }
            if let Err(error) = workspaces::ensure_worktree_attachment_mounts_if_materialized(
                self.state.as_ref(),
                &workspace,
                worktree,
            )
            .await
            {
                tracing::warn!(task_id = %task_id.0, "attachment mounts failed: {error:?}");
            }
            if let Err(error) = workspaces::spawn_worktree_bootstrap(
                self.state.clone(),
                workspace.clone(),
                worktree.clone(),
            )
            .await
            {
                tracing::warn!(task_id = %task_id.0, "worktree bootstrap failed: {error:?}");
            }
            if let Err(error) = workspaces::ensure_task_commit_hook(
                self.state.as_ref(),
                &workspace,
                worktree,
                task_id,
            )
            .await
            {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to configure vcs hooks: {error:#}"
                );
            }
        }

        let updated = store
            .unarchive_task(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?;
        if !updated {
            return Err(TaskLifecycleError::NotFound);
        }
        let task = store
            .get_task_with_activity(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?
            .ok_or(TaskLifecycleError::NotFound)?;
        let _ = self
            .state
            .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Unarchived)
            .await;
        if let Err(error) = self.state.emit_workspace_task_upsert(task_id).await {
            tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {error:?}");
        }
        self.state
            .emit_workspace_archived_task_delete(task.workspace_id, task_id)
            .await;
        for session_id in session_ids {
            self.state
                .workspaces
                .workspace_active_snapshot
                .remove_session(session_id)
                .await;
            self.state.refresh_session_head_cache(session_id).await;
        }
        Ok(task)
    }

    pub async fn delete_task(&self, task_id: TaskId) -> Result<(), TaskLifecycleError> {
        let Some((store, task, workspace)) = self.load_task_context(task_id).await? else {
            return Err(TaskLifecycleError::NotFound);
        };
        self.delete_loaded_task_with_cleanup(&store, &workspace, &task)
            .await
    }

    pub(in crate::daemon) async fn delete_loaded_task_with_cleanup(
        &self,
        store: &Store,
        workspace: &Workspace,
        task: &Task,
    ) -> Result<(), TaskLifecycleError> {
        let task_id = task.id;
        let sessions = store
            .list_all_sessions_for_task(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?;
        let cleanup_targets = collect_task_delete_cleanup_targets(
            self.state.as_ref(),
            store,
            workspace,
            task,
            &sessions,
        )
        .await;

        for session in &sessions {
            self.state.cleanup_session(session.id).await;
        }
        let deleted = store
            .delete_task(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?;
        if !deleted {
            return Err(TaskLifecycleError::NotFound);
        }

        let cleanup_errors = workspaces::cleanup_task_worktrees(
            self.state.as_ref(),
            workspace,
            task_id,
            &cleanup_targets,
            BranchCleanupErrorMode::BestEffort,
        )
        .await;
        if !cleanup_errors.is_empty() {
            tracing::warn!(
                task_id = %task_id.0,
                cleanup_errors = cleanup_errors.len(),
                "delete cleanup had errors after task row removal"
            );
        }
        let cleanup_succeeded = cleanup_errors.is_empty();
        delete_unused_worktree_records_after_cleanup(
            self.state.as_ref(),
            store,
            task,
            &cleanup_targets,
            cleanup_succeeded,
        )
        .await;
        if let Err(error) = self
            .state
            .global_store()
            .delete_workspace_task_index(task_id)
            .await
        {
            tracing::warn!(task_id = %task_id.0, "failed to delete workspace task index: {error:#}");
        }
        for session in sessions {
            if let Err(error) = self
                .state
                .global_store()
                .delete_workspace_session_index(session.id)
                .await
            {
                tracing::warn!(
                    task_id = %task_id.0,
                    session_id = %session.id.0,
                    "failed to delete workspace session index: {error:#}"
                );
            }
        }
        self.state
            .emit_workspace_task_delete(task.workspace_id, task_id)
            .await;
        if task.archived_at.is_some() {
            self.state
                .emit_workspace_archived_task_delete(task.workspace_id, task_id)
                .await;
        }
        Ok(())
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

async fn collect_task_delete_cleanup_targets(
    state: &crate::daemon::DaemonState,
    store: &Store,
    workspace: &Workspace,
    task: &Task,
    sessions: &[Session],
) -> Vec<TaskWorktreeCleanupTarget> {
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut cleanup_targets = Vec::new();
    for worktree_id in &worktree_ids {
        let other_active = match store
            .count_active_tasks_for_worktree(*worktree_id, Some(task.id))
            .await
        {
            Ok(count) => count > 0,
            Err(error) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check worktree usage: {error:#}"
                );
                true
            }
        };
        if other_active {
            continue;
        }
        let other_tasks = match store
            .count_tasks_for_worktree(*worktree_id, Some(task.id))
            .await
        {
            Ok(count) => count > 0,
            Err(error) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check total worktree usage: {error:#}"
                );
                true
            }
        };
        let worktree = match store.get_worktree(*worktree_id).await {
            Ok(Some(worktree)) => worktree,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for delete cleanup: {error:#}"
                );
                continue;
            }
        };
        let sandbox_binding = match store.get_sandbox_binding(*worktree_id).await {
            Ok(binding) => binding,
            Err(error) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load sandbox binding for delete cleanup: {error:#}"
                );
                None
            }
        };
        cleanup_targets.push(TaskWorktreeCleanupTarget {
            managed_root: workspaces::managed_worktree_root(state, workspace, &worktree),
            sandbox_binding,
            worktree,
            destroy_worktree_on_cleanup: !other_tasks,
        });
    }
    cleanup_targets
}

async fn delete_unused_worktree_records_after_cleanup(
    state: &crate::daemon::DaemonState,
    store: &Store,
    task: &Task,
    cleanup_targets: &[TaskWorktreeCleanupTarget],
    cleanup_succeeded: bool,
) {
    if !cleanup_succeeded {
        return;
    }
    for target in cleanup_targets {
        if !target.destroy_worktree_on_cleanup {
            continue;
        }
        let deleted_worktree_row = match store.delete_worktree(target.worktree.id).await {
            Ok(deleted) => deleted,
            Err(error) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %target.worktree.id.0,
                    "failed to delete worktree row after task delete: {error:#}"
                );
                false
            }
        };
        if !deleted_worktree_row {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %target.worktree.id.0,
                "skipping worktree index deletion because worktree row was not deleted"
            );
            continue;
        }
        if let Err(error) = state
            .global_store()
            .delete_workspace_worktree_index(target.worktree.id)
            .await
        {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %target.worktree.id.0,
                "failed to delete worktree index after task delete: {error:#}"
            );
        }
    }
}

async fn load_archive_worktrees(
    store: &Store,
    task: &Task,
    sessions: &[Session],
) -> Result<Vec<Worktree>, TaskLifecycleError> {
    let mut worktree_ids: HashSet<WorktreeId> =
        sessions.iter().map(|session| session.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut seen = HashSet::new();
    let mut worktrees = Vec::new();
    for worktree_id in worktree_ids {
        if !seen.insert(worktree_id) {
            continue;
        }
        let worktree = store
            .get_worktree(worktree_id)
            .await
            .map_err(TaskLifecycleError::Internal)?
            .ok_or(TaskLifecycleError::NotFound)?;
        worktrees.push(worktree);
    }
    Ok(worktrees)
}

struct UnarchiveWorktreePlan {
    session_ids: Vec<SessionId>,
    managed_worktrees: Vec<(Worktree, PathBuf)>,
    worktrees: Vec<Worktree>,
}

async fn load_unarchive_worktree_plan(
    handle: &TasksHandle,
    store: &Store,
    workspace: &Workspace,
    task: &Task,
) -> Result<UnarchiveWorktreePlan, TaskLifecycleError> {
    let task_id = task.id;
    let mut seen = HashSet::new();
    let mut managed_worktrees = Vec::new();
    let mut worktrees = Vec::new();
    let sessions = store
        .list_sessions_for_task(task_id)
        .await
        .map_err(TaskLifecycleError::Internal)?;
    let session_ids: Vec<SessionId> = sessions.iter().map(|session| session.id).collect();
    let mut worktree_ids: HashSet<WorktreeId> =
        sessions.iter().map(|session| session.worktree_id).collect();
    if let Some(primary) = task.primary_worktree_id {
        worktree_ids.insert(primary);
    }
    for worktree_id in worktree_ids {
        let worktree = store
            .get_worktree(worktree_id)
            .await
            .map_err(TaskLifecycleError::Internal)?
            .ok_or(TaskLifecycleError::NotFound)?;
        if let Some(root) =
            workspaces::managed_worktree_root(handle.state.as_ref(), workspace, &worktree)
        {
            if seen.insert(worktree.id) {
                managed_worktrees.push((worktree.clone(), root));
            }
        }
        worktrees.push(worktree);
    }

    Ok(UnarchiveWorktreePlan {
        session_ids,
        managed_worktrees,
        worktrees,
    })
}
