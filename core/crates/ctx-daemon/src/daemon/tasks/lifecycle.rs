use ctx_core::ids::TaskId;
use ctx_core::models::{Task, TaskDeltaKind, Workspace};
use ctx_store::Store;
use ctx_task_service::lifecycle::{self, LifecycleCleanupTarget};
use ctx_worktree_vcs_service::ensure_worktree_attached;

use crate::daemon::handle::TasksHandle;
use crate::daemon::workspaces::{self, BranchCleanupErrorMode, TaskWorktreeCleanupTarget};
use crate::daemon::DaemonState;

pub use ctx_task_service::lifecycle::TaskLifecycleError;

pub struct ArchiveTaskOutcome {
    pub task: Task,
    pub cleanup_failed: bool,
}

impl TasksHandle {
    pub async fn archive_task(
        &self,
        task_id: TaskId,
    ) -> Result<ArchiveTaskOutcome, TaskLifecycleError> {
        let Some((store, task, workspace)) = self.load_task_context(task_id).await? else {
            return Err(TaskLifecycleError::NotFound);
        };
        let plan = lifecycle::load_archive_task_plan(&store, &task).await?;
        for session in &plan.sessions {
            self.state.cleanup_session(session.id).await;
        }

        let task = lifecycle::archive_task_record(&store, task_id).await?;
        let service_cleanup_targets =
            lifecycle::collect_archive_cleanup_targets(&store, task_id, &plan.worktrees).await;
        let cleanup_targets =
            daemon_cleanup_targets(self.state.as_ref(), &workspace, &service_cleanup_targets);
        let errors = workspaces::cleanup_task_worktrees(
            self.state.as_ref(),
            &workspace,
            task_id,
            &cleanup_targets,
            BranchCleanupErrorMode::Report,
        )
        .await;
        let cleanup_failed = !errors.is_empty();
        if cleanup_failed {
            tracing::warn!(
                task_id = %task_id.0,
                "archive cleanup had errors after task state was persisted"
            );
        }
        let _ = self
            .state
            .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Archived)
            .await;
        if let Err(error) = self.state.emit_workspace_task_upsert(task_id).await {
            tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {error:?}");
        }
        for session_id in plan.session_ids {
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
        let plan = lifecycle::load_unarchive_worktree_plan(&store, &task).await?;

        for worktree in &plan.worktrees {
            let Some(root) =
                workspaces::managed_worktree_root(self.state.as_ref(), &workspace, worktree)
            else {
                continue;
            };
            let branch = worktree.git_branch.as_deref().unwrap_or_default();
            ensure_worktree_attached(
                &workspace.root_path,
                &root,
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

        for worktree in &plan.worktrees {
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

        let task = lifecycle::unarchive_task_record(&store, task_id).await?;
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
        for session_id in plan.session_ids {
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
        let plan = lifecycle::load_delete_task_plan(store, task).await?;
        let cleanup_targets =
            daemon_cleanup_targets(self.state.as_ref(), workspace, &plan.cleanup_targets);

        for session in &plan.sessions {
            self.state.cleanup_session(session.id).await;
        }
        lifecycle::delete_task_record(store, task_id).await?;

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
        let deleted_worktree_ids = lifecycle::delete_unused_worktree_records_after_cleanup(
            store,
            task,
            &plan.cleanup_targets,
            cleanup_errors.is_empty(),
        )
        .await;
        for worktree_id in deleted_worktree_ids {
            if let Err(error) = self
                .state
                .global_store()
                .delete_workspace_worktree_index(worktree_id)
                .await
            {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to delete worktree index after task delete: {error:#}"
                );
            }
        }
        if let Err(error) = self
            .state
            .global_store()
            .delete_workspace_task_index(task_id)
            .await
        {
            tracing::warn!(task_id = %task_id.0, "failed to delete workspace task index: {error:#}");
        }
        for session in plan.sessions {
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
}

fn daemon_cleanup_targets(
    state: &DaemonState,
    workspace: &Workspace,
    targets: &[LifecycleCleanupTarget],
) -> Vec<TaskWorktreeCleanupTarget> {
    targets
        .iter()
        .map(|target| TaskWorktreeCleanupTarget {
            managed_root: workspaces::managed_worktree_root(state, workspace, &target.worktree),
            sandbox_binding: target.sandbox_binding.clone(),
            worktree: target.worktree.clone(),
            destroy_worktree_on_cleanup: target.destroy_worktree_on_cleanup,
        })
        .collect()
}
