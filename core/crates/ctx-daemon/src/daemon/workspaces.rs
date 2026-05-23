use std::path::PathBuf;
use std::sync::Arc;

use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{SandboxBinding, VcsKind, Workspace, WorkspaceAttachment, Worktree};
use ctx_observability::telemetry::TelemetryEvent;
use ctx_settings_model::ExecutionSettings;
use ctx_store::Store;
use ctx_workspace_attachments::AttachmentConfig;
use ctx_workspace_container::WorkspaceContainerStatus;

use super::handle::WorkspacesHandle;
use crate::daemon::{settings, WorkspaceStoreAccessError};

mod active_snapshot_state;
mod app_state;
pub mod attachments;
mod cache_stats;
mod deletion;
mod diff_exec;
mod execution;
mod execution_config;
mod file_completions;
mod harness_container;
mod hydration;
mod management;
mod model_preferences;
mod prompt_bootstrap_config;
mod provider_model_preferences_route;
mod retry;
mod route_config;
mod route_contract;
mod run_archive;
mod runtime;
mod sandbox_binding;
pub mod stream;
pub mod vcs_hooks;
mod workspace_file_completions_route;
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
    WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError,
};
pub use retry::retry_global_index_write;
pub(in crate::daemon::workspaces) use route_config::workspace_store_route_error;
pub(in crate::daemon::workspaces) use route_config::WorkspaceRouteError;
pub use run_archive::RunArchiveIngestError;
pub use sandbox_binding::rematerialize_sandbox_binding_for_worktree;
pub use stream::{WorkspaceStreamAccessError, WorkspaceStreamRouteAdmission};
pub use vcs_hooks::{cleanup_workspace_hooks, cleanup_worktree_hooks, ensure_task_commit_hook};
pub use worktree_bootstrap::spawn_worktree_bootstrap;
pub use worktree_cleanup::{
    cleanup_task_worktrees, managed_worktree_root, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};
pub use worktree_provision::{persist_provisioned_worktree, provision_worktree_for_execution};

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
}
