use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

use ctx_core::ids::{MergeQueueEntryId, RunId, SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    MergeQueueEntry, MergeQueueRun, RunArchiveIngestBatch, RunArchiveIngestCursor, SandboxBinding,
    SessionHeadDelta, SessionHeadSnapshot, SessionSummaryDelta, VcsKind, Workspace,
    WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, WorkspaceActiveSnapshotClientMessage,
    WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotStreamMessage, WorkspaceAttachment,
    Worktree,
};
use ctx_observability::telemetry::TelemetryEvent;
use ctx_settings_model::ExecutionSettings;
use ctx_store::Store;
use ctx_workspace_attachments::AttachmentConfig;
use ctx_workspace_config as workspace_config;
use ctx_workspace_container::WorkspaceContainerStatus;

use super::handle::WorkspacesHandle;
use crate::daemon::{settings, WorkspaceStoreAccessError, WorkspaceStreamHandle};
use ctx_workspace_active_snapshot::{SessionReplayCursor, WorkspaceActiveSubscriptionState};

mod active_snapshot_state;
mod app_state;
pub mod attachments;
mod cache_stats;
mod deletion;
mod diff_exec;
mod execution;
mod file_completions;
mod harness_container;
mod hydration;
mod model_preferences;
mod retry;
mod runtime;
mod sandbox_binding;
pub mod stream;
pub mod vcs_hooks;
mod worktree_bootstrap;
mod worktree_cleanup;
mod worktree_provision;

pub use active_snapshot_state::load_workspace_active_snapshot_state;
pub use attachments::ensure_worktree_attachment_mounts_if_materialized;
pub use cache_stats::WorkspaceCacheDebugStats;
pub use deletion::{delete_workspace, WorkspaceDeleteError};
pub use diff_exec::{diff_worktree_for_session, diff_worktree_summary_for_session};
pub use execution::{
    execution_environment_from_settings, resolve_existing_worktree_execution,
    ResolvedExistingWorktreeExecution,
};
pub use file_completions::{
    complete_files_for_session, complete_files_for_workspace, FileCompletionsError,
    FileCompletionsErrorKind,
};
pub use harness_container::{
    ensure_workspace_harness_container, stop_workspace_harness_container,
    workspace_harness_container_status, WorkspaceHarnessContainerError,
};
pub use hydration::{WorkspaceHydrationError, WorkspaceHydrationErrorKind};
pub use model_preferences::{
    get_workspace_provider_model_preference, set_workspace_provider_model_preference,
    update_workspace_provider_preferred_model_id, WorkspaceProviderModelPreference,
    WorkspaceProviderModelPreferenceError,
};
pub use retry::retry_global_index_write;
pub use sandbox_binding::rematerialize_sandbox_binding_for_worktree;
pub use vcs_hooks::{cleanup_workspace_hooks, cleanup_worktree_hooks, ensure_task_commit_hook};
pub use worktree_bootstrap::spawn_worktree_bootstrap;
pub use worktree_cleanup::{
    cleanup_task_worktrees, managed_worktree_root, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};
pub use worktree_provision::{persist_provisioned_worktree, provision_worktree_for_execution};

#[derive(Debug)]
pub enum RunArchiveIngestError {
    WorkspaceNotFound,
    AcknowledgementConflict(&'static str),
    Internal(anyhow::Error),
}

impl WorkspacesHandle {
    pub async fn list_workspaces(&self) -> anyhow::Result<Vec<Workspace>> {
        self.state.global_store().list_workspaces().await
    }

    pub async fn get_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<Workspace>> {
        self.state.global_store().get_workspace(workspace_id).await
    }

    pub async fn workspace_exists(&self, workspace_id: WorkspaceId) -> anyhow::Result<bool> {
        self.state
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map(|workspace| workspace.is_some())
    }

    pub async fn create_workspace(
        &self,
        name: String,
        root_path: String,
        vcs_kind: VcsKind,
    ) -> anyhow::Result<Workspace> {
        self.state
            .global_store()
            .create_workspace(name, root_path, vcs_kind)
            .await
    }

    pub async fn list_workspace_attachments(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceAttachment>, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        store
            .list_workspace_attachments(workspace_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_workspace_primary_branch(
        &self,
        workspace_id: WorkspaceId,
        primary_branch: &str,
    ) -> anyhow::Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        ctx_workspace_config::update_primary_branch(&store, primary_branch).await
    }

    pub async fn load_workspace_primary_branch_config(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<String>, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::load_primary_branch(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_workspace_primary_branch_config(
        &self,
        workspace: &Workspace,
        primary_branch: &str,
    ) -> anyhow::Result<()> {
        let store = self.store_for_workspace(workspace.id).await?;
        workspace_config::update_primary_branch(&store, primary_branch).await?;
        let worktrees = store.list_worktrees(workspace.id).await?;
        for worktree in worktrees {
            if let Err(error) = self.refresh_worktree_vcs_snapshot(&worktree, true).await {
                tracing::warn!(
                    workspace_id = %workspace.id.0,
                    worktree_id = %worktree.id.0,
                    "failed to refresh worktree vcs after primary branch update: {error:#}"
                );
            }
        }
        Ok(())
    }

    pub async fn load_workspace_merge_queue_config(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<workspace_config::MergeQueueConfig, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::load_merge_queue_config(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_workspace_merge_queue_config(
        &self,
        workspace_id: WorkspaceId,
        update: workspace_config::MergeQueueConfigUpdate,
    ) -> anyhow::Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        let was_enabled = workspace_config::load_merge_queue_config(&store)
            .await?
            .enabled;
        workspace_config::update_merge_queue_config(&store, update).await?;
        let now_enabled = workspace_config::load_merge_queue_config(&store)
            .await?
            .enabled;
        if !was_enabled && now_enabled {
            self.schedule_workspace_merge_queue_if_enabled_and_queued(workspace_id)
                .await?;
        } else if was_enabled && !now_enabled {
            self.cancel_queued_entries_for_disabled_workspace(&store, workspace_id)
                .await?;
        }
        Ok(())
    }

    pub async fn list_merge_queue_entries_for_route(
        &self,
        workspace_id: WorkspaceId,
        limit: Option<i64>,
    ) -> Result<Vec<MergeQueueEntry>, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        store
            .list_merge_queue_entries(workspace_id, limit)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn latest_merge_queue_run_for_route(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> Result<Option<(Workspace, MergeQueueRun)>, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        let Some(workspace) = store
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?
        else {
            return Ok(None);
        };
        let run = store
            .get_latest_merge_queue_run(entry_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        Ok(run.map(|run| (workspace, run)))
    }

    pub async fn load_workspace_execution_override(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<workspace_config::ExecutionSettingsOverride>, WorkspaceStoreAccessError>
    {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::load_execution_settings_override(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_workspace_execution_config(
        &self,
        workspace_id: WorkspaceId,
        update: workspace_config::ExecutionConfigUpdate,
    ) -> anyhow::Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        workspace_config::update_execution_config(&store, update).await
    }

    pub async fn load_worktree_bootstrap_config(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<workspace_config::WorktreeBootstrapConfig>, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::load_worktree_bootstrap_config(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_worktree_bootstrap_config(
        &self,
        workspace_id: WorkspaceId,
        update: workspace_config::WorktreeBootstrapConfigUpdate,
    ) -> anyhow::Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        workspace_config::update_worktree_bootstrap_config(&store, update).await
    }

    pub async fn load_agent_system_prompt_append(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<workspace_config::AgentSystemPromptAppendConfig, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::load_agent_system_prompt_append(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_agent_system_prompt_append(
        &self,
        workspace_id: WorkspaceId,
        system_prompt_append: Option<String>,
    ) -> Result<workspace_config::AgentSystemPromptAppendConfig, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::update_agent_system_prompt_append(&store, system_prompt_append)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        workspace_config::load_agent_system_prompt_append(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn load_subagent_system_prompt_append(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<workspace_config::SubagentSystemPromptAppendConfig, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::load_subagent_system_prompt_append(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_subagent_system_prompt_append(
        &self,
        workspace_id: WorkspaceId,
        system_prompt_append: Option<String>,
    ) -> Result<workspace_config::SubagentSystemPromptAppendConfig, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::update_subagent_system_prompt_append(&store, system_prompt_append)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        workspace_config::load_subagent_system_prompt_append(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn get_worktree_with_live_root(
        &self,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<Option<Worktree>> {
        let Some(store) = self.worktree_store_or_none(worktree_id).await? else {
            return Ok(None);
        };
        let Some(mut worktree) = store.get_worktree(worktree_id).await? else {
            return Ok(None);
        };
        worktree.root_path = self
            .resolve_live_worktree_root(&worktree)
            .await?
            .to_string_lossy()
            .to_string();
        Ok(Some(worktree))
    }

    pub async fn get_worktree_bootstrap_log_path(
        &self,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<Option<String>> {
        let Some(store) = self.worktree_store_or_none(worktree_id).await? else {
            return Ok(None);
        };
        let Some(worktree) = store.get_worktree(worktree_id).await? else {
            return Ok(None);
        };
        Ok(worktree.bootstrap_log_path)
    }

    pub async fn build_run_archive_ingest_batch(
        &self,
        workspace_id: WorkspaceId,
        run_id: RunId,
        max_items: u32,
    ) -> Result<Option<RunArchiveIngestBatch>, RunArchiveIngestError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(run_archive_workspace_store_error)?;
        let batch = store
            .build_run_archive_ingest_batch(run_id, max_items)
            .await
            .map_err(RunArchiveIngestError::Internal)?;
        Ok(batch.filter(|batch| batch.run.workspace_id == workspace_id))
    }

    pub async fn acknowledge_run_archive_ingest_batch(
        &self,
        workspace_id: WorkspaceId,
        run_id: RunId,
        max_items: u32,
        batch: RunArchiveIngestBatch,
    ) -> Result<RunArchiveIngestCursor, RunArchiveIngestError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(run_archive_workspace_store_error)?;
        let cursor = store
            .get_run_archive_ingest_cursor(run_id)
            .await
            .map_err(RunArchiveIngestError::Internal)?;
        let current_watermark = cursor
            .as_ref()
            .map(|cursor| cursor.watermark)
            .unwrap_or_default();
        if batch.from != current_watermark {
            return Err(RunArchiveIngestError::AcknowledgementConflict(
                "archive ingest acknowledgement is stale for the current cursor",
            ));
        }
        let Some(mut expected_batch) = store
            .build_run_archive_ingest_batch_after(run_id, batch.from, max_items, cursor.is_none())
            .await
            .map_err(RunArchiveIngestError::Internal)?
        else {
            return Err(RunArchiveIngestError::AcknowledgementConflict(
                "archive ingest acknowledgement does not match an available batch",
            ));
        };
        expected_batch.created_at = batch.created_at;
        if expected_batch != batch {
            return Err(RunArchiveIngestError::AcknowledgementConflict(
                "archive ingest acknowledgement does not match the current batch",
            ));
        }
        store
            .acknowledge_run_archive_ingest_batch(&batch)
            .await
            .map_err(RunArchiveIngestError::Internal)
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.state.store_for_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, WorkspaceStoreAccessError> {
        self.state.existing_workspace_store(workspace_id).await
    }

    async fn worktree_store_or_none(
        &self,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<Option<Store>> {
        let Some(workspace_id) = self
            .state
            .global_store()
            .get_workspace_id_for_worktree(worktree_id)
            .await?
        else {
            return Ok(None);
        };
        match self.existing_workspace_store(workspace_id).await {
            Ok(store) => Ok(Some(store)),
            Err(WorkspaceStoreAccessError::NotFound) => Ok(None),
            Err(WorkspaceStoreAccessError::Unavailable(error)) => Err(error),
        }
    }

    pub async fn record_workspace_registered(&self) {
        self.state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::workspace_registered())
            .await;
    }

    pub async fn record_workspace_opened(&self) {
        self.state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::workspace_opened())
            .await;
    }

    pub async fn delete_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceDeleteError> {
        delete_workspace(&self.state, workspace_id).await
    }

    pub async fn load_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceActiveSnapshot, WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await?;
        crate::daemon::merge_queue::activate_workspace_merge_queue(&self.state, workspace_id).await;
        let snapshot = self
            .state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(workspace_id, i64::MAX)
            .await;
        self.state
            .cache_workspace_active_snapshot(snapshot.clone())
            .await;
        Ok(snapshot)
    }

    pub async fn load_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceActiveHeadBatch, WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await?;
        crate::daemon::merge_queue::activate_workspace_merge_queue(&self.state, workspace_id).await;
        let heads = self
            .state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await;
        self.state.cache_workspace_active_heads(heads.clone()).await;
        Ok(heads)
    }

    pub async fn effective_execution_settings(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<ExecutionSettings> {
        crate::daemon::execution_effective::effective_execution_settings(
            self.state.as_ref(),
            workspace_id,
        )
        .await
    }

    pub async fn effective_execution_settings_classified(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<ExecutionSettings, ctx_settings_service::EffectiveExecutionSettingsError> {
        crate::daemon::execution_effective::effective_execution_settings_classified(
            self.state.as_ref(),
            workspace_id,
        )
        .await
    }

    pub async fn resolve_existing_worktree_execution(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<ResolvedExistingWorktreeExecution> {
        resolve_existing_worktree_execution(&self.state, store, workspace, worktree_id).await
    }

    pub async fn provision_worktree_for_execution(
        &self,
        workspace: &Workspace,
        worktree_id: WorktreeId,
        base_commit_sha: &str,
        branch_name: &str,
        effective: &ExecutionSettings,
    ) -> anyhow::Result<(PathBuf, Option<SandboxBinding>)> {
        provision_worktree_for_execution(
            &self.state,
            workspace,
            worktree_id,
            base_commit_sha,
            branch_name,
            effective,
        )
        .await
    }

    pub async fn persist_provisioned_worktree(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree: Worktree,
        sandbox_binding: Option<SandboxBinding>,
    ) -> anyhow::Result<()> {
        persist_provisioned_worktree(&self.state, store, workspace, worktree, sandbox_binding)
            .await
            .map(|_| ())
    }

    pub fn managed_worktree_root(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Option<PathBuf> {
        managed_worktree_root(self.state.as_ref(), workspace, worktree)
    }

    pub async fn resolve_live_worktree_root(&self, worktree: &Worktree) -> anyhow::Result<PathBuf> {
        Ok(
            ctx_worktree_data_plane::resolve_worktree_data_plane_with_host(
                self.state.as_ref(),
                worktree,
            )
            .await?
            .live_worktree_root,
        )
    }

    pub fn worktree_bootstrap_logs_root(&self) -> PathBuf {
        ctx_observability::logs::logs_dir(&self.state.core.data_root).join("worktree-bootstrap")
    }

    pub async fn cleanup_task_worktrees(
        &self,
        workspace: &Workspace,
        task_id: TaskId,
        targets: &[TaskWorktreeCleanupTarget],
        branch_cleanup_error_mode: BranchCleanupErrorMode,
    ) -> Vec<anyhow::Error> {
        cleanup_task_worktrees(
            self.state.as_ref(),
            workspace,
            task_id,
            targets,
            branch_cleanup_error_mode,
        )
        .await
    }

    pub async fn upsert_workspace_attachment(
        &self,
        workspace_id: WorkspaceId,
        cfg: AttachmentConfig,
    ) -> anyhow::Result<WorkspaceAttachment> {
        attachments::upsert_workspace_attachment(self.state.as_ref(), workspace_id, cfg).await
    }

    pub async fn delete_workspace_attachment(
        &self,
        workspace_id: WorkspaceId,
        kind: ctx_core::models::WorkspaceAttachmentKind,
        name: &str,
    ) -> anyhow::Result<bool> {
        attachments::delete_workspace_attachment(self.state.as_ref(), workspace_id, kind, name)
            .await
    }

    pub async fn sync_workspace_attachments(
        &self,
        workspace: &Workspace,
        refresh: bool,
    ) -> anyhow::Result<Vec<WorkspaceAttachment>> {
        let attachments =
            attachments::sync_workspace_attachments(Arc::clone(&self.state), workspace, refresh)
                .await?;
        let _ = attachments::ensure_workspace_attachments_for_worktrees_with_attachments(
            self.state.as_ref(),
            workspace,
            &attachments,
            false,
            false,
        )
        .await;
        Ok(attachments)
    }

    pub async fn ensure_task_commit_hook(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        ensure_task_commit_hook(self.state.as_ref(), workspace, worktree, task_id).await
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> anyhow::Result<()> {
        self.state.emit_workspace_task_upsert(task_id).await
    }

    pub async fn workspace_harness_container_status(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceContainerStatus>, WorkspaceHarnessContainerError> {
        workspace_harness_container_status(&self.state, workspace_id).await
    }

    pub async fn stop_workspace_harness_container(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceHarnessContainerError> {
        stop_workspace_harness_container(&self.state, workspace_id).await
    }

    pub async fn ensure_workspace_harness_container(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceHarnessContainerError> {
        ensure_workspace_harness_container(&self.state, workspace_id).await
    }

    pub async fn get_workspace_provider_model_preference(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
    ) -> Result<WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError> {
        get_workspace_provider_model_preference(&self.state, workspace_id, provider_id).await
    }

    pub async fn set_workspace_provider_model_preference(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
        preferred_model_id: Option<String>,
    ) -> Result<WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError> {
        set_workspace_provider_model_preference(
            &self.state,
            workspace_id,
            provider_id,
            preferred_model_id,
        )
        .await
    }

    pub async fn complete_files_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        query: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<String>, FileCompletionsError> {
        complete_files_for_workspace(&self.state, workspace_id, query, limit).await
    }

    pub async fn load_settings(&self) -> anyhow::Result<ctx_settings_model::Settings> {
        settings::load_settings(self.state.as_ref()).await
    }

    #[cfg(target_os = "macos")]
    pub fn shared_vm_container_runtime_available(&self) -> bool {
        ctx_harness_runtime::local_runtime_available(
            &self.state.core.data_root,
            &ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
        )
    }

    pub async fn refresh_worktree_vcs_snapshot(
        &self,
        worktree: &Worktree,
        force_emit: bool,
    ) -> anyhow::Result<()> {
        crate::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
            &self.state,
            worktree,
            force_emit,
        )
        .await
    }

    pub async fn schedule_workspace_merge_queue_if_enabled_and_queued(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<bool> {
        crate::daemon::merge_queue::schedule_workspace_if_enabled_and_queued(
            &self.state,
            workspace_id,
        )
        .await
    }

    pub async fn cancel_queued_entries_for_disabled_workspace(
        &self,
        store: &Store,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        crate::daemon::merge_queue::cancel_queued_entries_for_disabled_workspace(
            &self.state,
            store,
            workspace_id,
        )
        .await
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<ctx_core::models::WorktreeVcsSnapshot> {
        self.state.get_worktree_vcs_snapshot(worktree_id).await
    }

    pub fn subscribe_worktree_vcs_events(
        &self,
    ) -> broadcast::Receiver<ctx_core::models::WorktreeVcsSnapshot> {
        self.state.subscribe_worktree_vcs_events()
    }

    pub async fn filter_workspace_worktree_ids(
        &self,
        workspace_id: WorkspaceId,
        worktree_ids: Vec<WorktreeId>,
    ) -> Vec<WorktreeId> {
        stream::filter_workspace_worktree_ids(&self.state, workspace_id, worktree_ids).await
    }

    pub async fn refresh_worktree_vcs_for_worktrees(
        &self,
        summary_worktree_ids: &[WorktreeId],
        detail_worktree_ids: &[WorktreeId],
    ) {
        stream::refresh_worktree_vcs_for_worktrees(
            &self.state,
            summary_worktree_ids,
            detail_worktree_ids,
        )
        .await;
    }

    pub async fn update_worktree_vcs_activity(
        &self,
        previous: &std::collections::HashSet<WorktreeId>,
        next: &std::collections::HashSet<WorktreeId>,
    ) {
        self.state
            .update_worktree_vcs_activity(previous, next)
            .await;
    }

    pub async fn update_worktree_vcs_open_panes(
        &self,
        previous: &std::collections::HashSet<WorktreeId>,
        next: &std::collections::HashSet<WorktreeId>,
    ) {
        self.state
            .update_worktree_vcs_open_panes(previous, next)
            .await;
    }

    #[cfg(test)]
    pub async fn is_worktree_vcs_active_for_test(&self, worktree_id: WorktreeId) -> bool {
        self.state.is_worktree_vcs_active(worktree_id).await
    }

    #[cfg(test)]
    pub async fn is_worktree_vcs_pane_open_for_test(&self, worktree_id: WorktreeId) -> bool {
        self.state.is_worktree_vcs_pane_open(worktree_id).await
    }

    pub async fn plan_workspace_vcs_subscription_update(
        &self,
        workspace_id: WorkspaceId,
        current: stream::WorkspaceVcsDemandState,
        summary_worktree_ids: Vec<WorktreeId>,
        detail_worktree_ids: Vec<WorktreeId>,
    ) -> stream::WorkspaceVcsSubscriptionPlan {
        stream::plan_workspace_vcs_subscription_update(
            &self.state,
            workspace_id,
            current,
            summary_worktree_ids,
            detail_worktree_ids,
        )
        .await
    }

    pub async fn plan_workspace_vcs_refresh(
        &self,
        workspace_id: WorkspaceId,
        worktree_ids: Vec<WorktreeId>,
        tier: ctx_core::models::WorktreeVcsStreamTier,
    ) -> stream::WorkspaceVcsRefreshPlan {
        stream::plan_workspace_vcs_refresh(&self.state, workspace_id, worktree_ids, tier).await
    }

    pub async fn release_workspace_vcs_demand(&self, demand: &stream::WorkspaceVcsDemandState) {
        stream::release_workspace_vcs_demand(&self.state, demand).await;
    }

    pub async fn record_workspace_vcs_stream_metric(&self, name: &str, value: u64) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("stream".to_string(), "workspace_vcs".to_string());
        let metric = ctx_observability::perf_telemetry::PerfMetric {
            name: name.to_string(),
            kind: ctx_observability::perf_telemetry::PerfMetricKind::Counter,
            unit: "count".to_string(),
            value: value as f64,
            labels,
        };
        self.state
            .telemetry
            .perf_telemetry
            .record_metric(metric, None, None, None)
            .await;
    }
}

fn run_archive_workspace_store_error(error: WorkspaceStoreAccessError) -> RunArchiveIngestError {
    match error {
        WorkspaceStoreAccessError::NotFound => RunArchiveIngestError::WorkspaceNotFound,
        WorkspaceStoreAccessError::Unavailable(error) => RunArchiveIngestError::Internal(error),
    }
}

impl WorkspaceStreamHandle {
    pub async fn workspace_exists(&self, workspace_id: WorkspaceId) -> anyhow::Result<bool> {
        self.state
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map(|workspace| workspace.is_some())
    }

    pub async fn subscribe_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Receiver<WorkspaceActiveSnapshotEvent> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .subscribe(workspace_id)
            .await
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
    }

    pub async fn activate_workspace_merge_queue(&self, workspace_id: WorkspaceId) {
        crate::daemon::merge_queue::activate_workspace_merge_queue(&self.state, workspace_id).await;
    }

    pub async fn load_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> (i64, i64) {
        load_workspace_active_snapshot_state(&self.state, workspace_id).await
    }

    pub async fn initial_stream_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> stream::WorkspaceStreamInitialState {
        stream::initial_stream_state(&self.state, workspace_id).await
    }

    pub async fn load_initial_snapshot_read_model(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<stream::WorkspaceStreamSnapshotReadModel, WorkspaceHydrationError> {
        stream::load_initial_snapshot_read_model(&self.state, workspace_id).await
    }

    pub async fn workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> WorkspaceActiveSnapshot {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(workspace_id, i64::MAX)
            .await
    }

    pub async fn workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> WorkspaceActiveHeadBatch {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await
    }

    pub async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        self.state
            .workspaces
            .workspace_active_snapshot
            .session_replay_cursor(workspace_id, session_id)
            .await
    }

    pub fn active_head_cursors_from_snapshot_read_model(
        &self,
        read_model: &stream::WorkspaceStreamSnapshotReadModel,
    ) -> HashMap<SessionId, SessionReplayCursor> {
        stream::active_head_cursors_from_snapshot_read_model(read_model)
    }

    pub async fn plan_workspace_stream_replay_program(
        &self,
        workspace_id: WorkspaceId,
        resolved_sessions: &[stream::WorkspaceStreamResolvedSession],
        live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
        include_initial_snapshot: bool,
    ) -> stream::WorkspaceStreamReplayProgram {
        stream::plan_workspace_stream_replay_program(
            &self.state,
            workspace_id,
            resolved_sessions,
            live_subscriptions,
            active_head_cursors,
            include_initial_snapshot,
        )
        .await
    }

    pub async fn plan_workspace_stream_replay_program_with_step_hook<H>(
        &self,
        workspace_id: WorkspaceId,
        resolved_sessions: &[stream::WorkspaceStreamResolvedSession],
        live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
        include_initial_snapshot: bool,
        step_hook: &mut H,
    ) -> Result<stream::WorkspaceStreamReplayProgram, H::Error>
    where
        H: stream::WorkspaceStreamReplayStepHook,
    {
        stream::plan_workspace_stream_replay_program_with_step_hook(
            &self.state,
            workspace_id,
            resolved_sessions,
            live_subscriptions,
            active_head_cursors,
            include_initial_snapshot,
            step_hook,
        )
        .await
    }

    pub fn accept_session_delta_cursor(
        &self,
        current: SessionReplayCursor,
        delta: &SessionHeadDelta,
    ) -> stream::WorkspaceStreamCursorAcceptance {
        stream::accept_session_delta_cursor(current, delta)
    }

    pub fn accept_session_head_cursor(
        &self,
        current: SessionReplayCursor,
        head: &SessionHeadSnapshot,
    ) -> stream::WorkspaceStreamCursorAcceptance {
        stream::accept_session_head_cursor(current, head)
    }

    pub fn is_session_head_delta_after_cursor(
        &self,
        delta: &SessionHeadDelta,
        cursor: SessionReplayCursor,
    ) -> bool {
        stream::is_session_head_delta_after_cursor(delta, cursor)
    }

    pub fn is_session_summary_delta_after_cursor(
        &self,
        delta: &SessionSummaryDelta,
        cursor: SessionReplayCursor,
    ) -> bool {
        stream::is_session_summary_delta_after_cursor(delta, cursor)
    }

    pub fn merge_replayed_and_live_subscription_cursors(
        &self,
        live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        replayed_subscriptions: HashMap<SessionId, SessionReplayCursor>,
    ) -> HashMap<SessionId, SessionReplayCursor> {
        stream::merge_replayed_and_live_subscription_cursors(
            live_subscriptions,
            replayed_subscriptions,
        )
    }

    pub fn finalize_workspace_stream_subscription_replay(
        &self,
        current_state: &WorkspaceActiveSubscriptionState,
        current_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        replayed_subscriptions: HashMap<SessionId, SessionReplayCursor>,
        transaction_sessions: &[stream::WorkspaceStreamResolvedSession],
    ) -> stream::WorkspaceStreamSubscriptionReplayFinalization {
        stream::finalize_workspace_stream_subscription_replay(
            current_state,
            current_subscriptions,
            replayed_subscriptions,
            transaction_sessions,
        )
    }

    pub async fn active_task_subscription_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        stream::active_task_subscription_cursor(&self.state, workspace_id, session_id).await
    }

    pub async fn apply_workspace_stream_subscription_event(
        &self,
        workspace_id: WorkspaceId,
        subscription_state: WorkspaceActiveSubscriptionState,
        subscriptions: HashMap<SessionId, SessionReplayCursor>,
        event: &WorkspaceActiveSnapshotEvent,
    ) -> stream::WorkspaceStreamSubscriptionEventApplication {
        stream::apply_workspace_stream_subscription_event(
            &self.state,
            workspace_id,
            subscription_state,
            subscriptions,
            event,
        )
        .await
    }

    pub async fn apply_workspace_stream_live_event(
        &self,
        workspace_id: WorkspaceId,
        subscription_state: WorkspaceActiveSubscriptionState,
        subscriptions: HashMap<SessionId, SessionReplayCursor>,
        event: WorkspaceActiveSnapshotEvent,
    ) -> stream::WorkspaceStreamLiveEventApplication {
        stream::apply_workspace_stream_live_event(
            &self.state,
            workspace_id,
            subscription_state,
            subscriptions,
            event,
        )
        .await
    }

    pub fn primary_session_id_for_active_task_event(
        &self,
        task: &ctx_core::models::WorkspaceActiveTaskSummary,
    ) -> SessionId {
        stream::primary_session_id_for_active_task_event(task)
    }

    pub fn event_snapshot_rev(&self, event: &WorkspaceActiveSnapshotEvent) -> Option<i64> {
        stream::event_snapshot_rev(event)
    }

    pub fn event_blocks_pending_replay(
        &self,
        event: &WorkspaceActiveSnapshotEvent,
        pending_replay_sessions: &HashSet<SessionId>,
        active_task_sessions: &HashMap<TaskId, SessionId>,
    ) -> bool {
        stream::event_blocks_pending_replay(event, pending_replay_sessions, active_task_sessions)
    }

    pub fn plan_workspace_stream_event_route(
        &self,
        subscription_state: &WorkspaceActiveSubscriptionState,
        event: WorkspaceActiveSnapshotEvent,
    ) -> stream::WorkspaceStreamEventRoutePlan {
        stream::plan_workspace_stream_event_route(subscription_state, event)
    }

    pub async fn resolve_workspace_active_snapshot_subscriptions(
        &self,
        workspace_id: WorkspaceId,
        message: WorkspaceActiveSnapshotClientMessage,
        existing: &HashMap<SessionId, SessionReplayCursor>,
    ) -> Result<
        stream::WorkspaceStreamSubscriptionPlan,
        stream::WorkspaceStreamSubscriptionResolutionError,
    > {
        stream::prepare_subscription_read_model(&self.state, workspace_id)
            .await
            .map_err(stream::WorkspaceStreamSubscriptionResolutionError::Hydration)?;
        let resolved = stream::resolve_workspace_active_snapshot_subscriptions(
            &self.state,
            workspace_id,
            message.clone(),
            existing,
        )
        .await
        .map_err(|_| stream::WorkspaceStreamSubscriptionResolutionError::Resolution)?;
        Ok(stream::plan_workspace_stream_subscription(
            &message, resolved, existing,
        ))
    }

    pub async fn plan_workspace_stream_subscription_transaction(
        &self,
        workspace_id: WorkspaceId,
        message: WorkspaceActiveSnapshotClientMessage,
        current_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        current_fingerprint: Option<&str>,
    ) -> Result<
        stream::WorkspaceStreamSubscriptionTransactionPlan,
        stream::WorkspaceStreamSubscriptionResolutionError,
    > {
        stream::prepare_subscription_read_model(&self.state, workspace_id)
            .await
            .map_err(stream::WorkspaceStreamSubscriptionResolutionError::Hydration)?;
        let resolved = stream::resolve_workspace_active_snapshot_subscriptions(
            &self.state,
            workspace_id,
            message.clone(),
            current_subscriptions,
        )
        .await
        .map_err(|_| stream::WorkspaceStreamSubscriptionResolutionError::Resolution)?;
        Ok(stream::plan_workspace_stream_subscription_transaction(
            &message,
            resolved,
            current_subscriptions,
            current_fingerprint,
        ))
    }

    pub async fn replay_session_events<F, Fut>(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        after_cursor: SessionReplayCursor,
        list_failpoint: &'static str,
        send_failpoint: Option<&'static str>,
        emit: F,
    ) -> Result<stream::ReplayOutcome, ()>
    where
        F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
        Fut: std::future::Future<Output = Result<(), ()>>,
    {
        stream::replay_session_events(
            &self.state,
            workspace_id,
            session_id,
            after_cursor,
            list_failpoint,
            send_failpoint,
            emit,
        )
        .await
    }

    pub async fn attach_session_pin(&self, session_id: SessionId) {
        self.state.attach_session(session_id).await;
    }

    pub async fn detach_session_pin(&self, session_id: SessionId) {
        self.state.detach_session(session_id).await;
    }

    pub async fn apply_workspace_stream_session_pin_changes(
        &self,
        pin_changes: &stream::WorkspaceStreamSessionPinChanges,
    ) {
        for session_id in &pin_changes.attach {
            self.state.attach_session(*session_id).await;
        }
        for session_id in &pin_changes.detach {
            self.state.detach_session(*session_id).await;
        }
    }

    pub async fn release_workspace_stream_session_pins<I>(&self, session_ids: I)
    where
        I: IntoIterator<Item = SessionId>,
    {
        for session_id in session_ids {
            self.state.detach_session(session_id).await;
        }
    }

    pub async fn emit_workspace_stream_incident(
        &self,
        event_name: &'static str,
        labels: &[(&'static str, serde_json::Value)],
    ) {
        let mut event = TelemetryEvent::daemon_incident(event_name)
            .with_source("workspace_stream")
            .with_property("has_workspace_scope", serde_json::json!(true));
        for (key, value) in labels {
            event = event.with_property(*key, value.clone());
        }
        self.state.telemetry.telemetry.emit(event).await;
    }

    pub async fn record_workspace_stream_receiver_drain(
        &self,
        queue_label: &'static str,
        event_count: usize,
        hit_limit: bool,
    ) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("queue_label".to_string(), queue_label.to_string());
        labels.insert(
            "hit_limit".to_string(),
            if hit_limit { "true" } else { "false" }.to_string(),
        );
        self.state
            .telemetry
            .perf_telemetry
            .record_metric(
                ctx_observability::perf_telemetry::PerfMetric {
                    name: "workspace.stream.receiver_drain_event_count".to_string(),
                    kind: ctx_observability::perf_telemetry::PerfMetricKind::Histogram,
                    unit: "count".to_string(),
                    value: event_count as f64,
                    labels,
                },
                None,
                None,
                None,
            )
            .await;
    }
}
