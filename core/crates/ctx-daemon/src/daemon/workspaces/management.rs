use std::path::{Path, PathBuf};

use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_core::models::{MergeQueueRun, Workspace, Worktree};
use ctx_store::Store;
use ctx_workspace_config as workspace_config;
use ctx_workspace_services::workspace_registration::validate_workspace_primary_branch;

use super::model_preferences::{
    get_workspace_provider_model_preference, set_workspace_provider_model_preference,
    WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError,
};
use super::route_config::{
    parse_workspace_route_id, provider_model_preference_error, workspace_store_error,
    AgentSystemPromptConfigRouteResponse, SubagentSystemPromptConfigRouteResponse,
    UpdateAgentSystemPromptConfigRouteRequest, UpdateSubagentSystemPromptConfigRouteRequest,
    UpdateWorkspaceExecutionConfigRequest, UpdateWorkspaceMergeQueueConfigRequest,
    UpdateWorkspacePrimaryBranchRequest, UpdateWorkspaceProviderModelPreferenceRouteRequest,
    UpdateWorktreeBootstrapConfigRequest, WorkspaceConfigUpdateResult,
    WorkspaceExecutionConfigSnapshot, WorkspaceMergeQueueConfigRouteResponse,
    WorkspacePrimaryBranchSnapshot, WorkspacePromptConfigRouteParams,
    WorkspaceProviderModelPreferenceRouteParams, WorkspaceProviderModelPreferenceRouteResponse,
    WorkspaceRouteError, WorkspaceWorktreeBootstrapConfigRouteResponse,
};
use crate::daemon::route_files::{read_text_route_file, RouteFileDownloadError, TextRouteDownload};
use crate::daemon::{settings, WorkspaceStoreAccessError, WorkspacesHandle};

impl WorkspacesHandle {
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

    pub async fn workspace_primary_branch_for_request(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let primary_branch = self
            .load_workspace_primary_branch_config(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?
            .ok_or_else(|| {
                WorkspaceRouteError::not_found("workspace primary branch is not configured")
            })?;
        Ok(WorkspacePrimaryBranchSnapshot { primary_branch })
    }

    pub async fn update_workspace_primary_branch_for_request(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspacePrimaryBranchRequest,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let workspace = self
            .state
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceRouteError::internal)?
            .ok_or_else(|| WorkspaceRouteError::not_found("workspace not found"))?;
        let primary_branch =
            validate_workspace_primary_branch(Path::new(&workspace.root_path), &req.primary_branch)
                .await
                .map_err(|error| WorkspaceRouteError::bad_request(error.message()))?;
        let store = self
            .existing_workspace_store(workspace.id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let primary_branch =
            workspace_config::update_and_load_primary_branch(&store, &primary_branch)
                .await
                .map_err(WorkspaceRouteError::internal)?;
        let worktrees = store
            .list_worktrees(workspace.id)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        for worktree in worktrees {
            if let Err(error) = self.refresh_worktree_vcs_snapshot(&worktree, true).await {
                tracing::warn!(
                    workspace_id = %workspace.id.0,
                    worktree_id = %worktree.id.0,
                    "failed to refresh worktree vcs after primary branch update: {error:#}"
                );
            }
        }
        Ok(WorkspacePrimaryBranchSnapshot { primary_branch })
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

    pub async fn download_merge_queue_entry_logs_for_route(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> Result<TextRouteDownload, RouteFileDownloadError> {
        self.get_workspace_merge_queue_entry(workspace_id, entry_id)
            .await
            .map_err(|_| RouteFileDownloadError::NotFound)?;
        let (workspace, run) = self
            .latest_merge_queue_run_for_route(workspace_id, entry_id)
            .await
            .map_err(|_| RouteFileDownloadError::Internal)?
            .ok_or(RouteFileDownloadError::NotFound)?;
        let Some(path) = run.log_path.as_deref() else {
            return Err(RouteFileDownloadError::NotFound);
        };
        if path.trim().is_empty() {
            return Err(RouteFileDownloadError::NotFound);
        }
        let log_root = PathBuf::from(&workspace.root_path)
            .join(".ctx")
            .join("merge-queue")
            .join("logs");
        read_text_route_file(
            Path::new(path),
            &log_root,
            format!("merge-queue-{}.log", entry_id.0),
        )
        .await
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

    pub async fn workspace_execution_config_for_request(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceExecutionConfigSnapshot, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let settings = settings::load_settings(self.state.as_ref())
            .await
            .map_err(WorkspaceRouteError::internal)?;
        match ctx_settings_service::workspace_execution_config_snapshot_for_loaded_settings(
            &settings, &store,
        )
        .await
        {
            Ok(snapshot) => Ok(snapshot),
            Err(
                ctx_settings_service::WorkspaceExecutionConfigSnapshotError::InvalidWorkspaceConfig(
                    error,
                ),
            ) => Err(WorkspaceRouteError::bad_request(error)),
            Err(ctx_settings_service::WorkspaceExecutionConfigSnapshotError::RequestOrPolicy(
                error,
            )) => Err(WorkspaceRouteError::from_request_or_policy_error(error)),
            Err(ctx_settings_service::WorkspaceExecutionConfigSnapshotError::Internal(error)) => {
                Err(WorkspaceRouteError::internal(error))
            }
        }
    }

    pub async fn update_workspace_execution_config_for_request(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspaceExecutionConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let update = workspace_config::parse_execution_config_update_input(
            req.environment.trim(),
            req.network_mode.as_deref(),
            req.allowlist,
            self.sandbox_runtime_available_for_execution_config(),
        )
        .map_err(WorkspaceRouteError::bad_request)?;
        let settings = settings::load_settings(self.state.as_ref())
            .await
            .map_err(WorkspaceRouteError::internal)?;
        ctx_settings_service::update_workspace_execution_config_for_loaded_settings(
            &settings, &store, update,
        )
        .await
        .map_err(|error| match error {
            ctx_settings_service::WorkspaceExecutionConfigUpdateError::RequestOrPolicy(error) => {
                WorkspaceRouteError::from_request_or_policy_error(error)
            }
            ctx_settings_service::WorkspaceExecutionConfigUpdateError::Persistence(error) => {
                WorkspaceRouteError::bad_request(error)
            }
        })?;
        Ok(WorkspaceConfigUpdateResult { ok: true })
    }

    pub async fn workspace_merge_queue_config_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceMergeQueueConfigRouteResponse, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let cfg = workspace_config::load_merge_queue_config(&store)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(cfg.into())
    }

    pub async fn update_workspace_merge_queue_config_for_route(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspaceMergeQueueConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let transition = workspace_config::update_merge_queue_config_with_transition(
            &store,
            req.into_merge_queue_config_update(),
        )
        .await
        .map_err(WorkspaceRouteError::from_request_or_policy_error)?;
        if !transition.was_enabled && transition.now_enabled {
            self.schedule_workspace_merge_queue_if_enabled_and_queued(workspace_id)
                .await
                .map_err(WorkspaceRouteError::from_request_or_policy_error)?;
        } else if transition.was_enabled && !transition.now_enabled {
            self.cancel_queued_entries_for_disabled_workspace(&store, workspace_id)
                .await
                .map_err(WorkspaceRouteError::from_request_or_policy_error)?;
        }
        Ok(WorkspaceConfigUpdateResult { ok: true })
    }

    pub async fn worktree_bootstrap_config_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceWorktreeBootstrapConfigRouteResponse, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let cfg = workspace_config::load_worktree_bootstrap_config(&store)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(cfg.into())
    }

    pub async fn update_worktree_bootstrap_config_for_route(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorktreeBootstrapConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        workspace_config::update_worktree_bootstrap_config(
            &store,
            req.into_worktree_bootstrap_config_update(),
        )
        .await
        .map_err(WorkspaceRouteError::bad_request)?;
        Ok(WorkspaceConfigUpdateResult { ok: true })
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
        workspace_config::update_and_load_agent_system_prompt_append(&store, system_prompt_append)
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
        workspace_config::update_and_load_subagent_system_prompt_append(
            &store,
            system_prompt_append,
        )
        .await
        .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn workspace_provider_model_preference_for_route(
        &self,
        params: WorkspaceProviderModelPreferenceRouteParams,
    ) -> Result<WorkspaceProviderModelPreferenceRouteResponse, WorkspaceRouteError> {
        let workspace_id = parse_workspace_route_id(&params.workspace_id)?;
        self.get_workspace_provider_model_preference(workspace_id, &params.provider_id)
            .await
            .map(Into::into)
            .map_err(provider_model_preference_error)
    }

    pub async fn update_workspace_provider_model_preference_for_route(
        &self,
        params: WorkspaceProviderModelPreferenceRouteParams,
        req: UpdateWorkspaceProviderModelPreferenceRouteRequest,
    ) -> Result<WorkspaceProviderModelPreferenceRouteResponse, WorkspaceRouteError> {
        let workspace_id = parse_workspace_route_id(&params.workspace_id)?;
        self.set_workspace_provider_model_preference(
            workspace_id,
            &params.provider_id,
            req.preferred_model_id,
        )
        .await
        .map(Into::into)
        .map_err(provider_model_preference_error)
    }

    pub async fn agent_system_prompt_config_for_route(
        &self,
        params: WorkspacePromptConfigRouteParams,
    ) -> Result<AgentSystemPromptConfigRouteResponse, WorkspaceRouteError> {
        let workspace_id = parse_workspace_route_id(&params.workspace_id)?;
        self.load_agent_system_prompt_append(workspace_id)
            .await
            .map(Into::into)
            .map_err(workspace_store_error)
    }

    pub async fn update_agent_system_prompt_config_for_route(
        &self,
        params: WorkspacePromptConfigRouteParams,
        req: UpdateAgentSystemPromptConfigRouteRequest,
    ) -> Result<AgentSystemPromptConfigRouteResponse, WorkspaceRouteError> {
        let workspace_id = parse_workspace_route_id(&params.workspace_id)?;
        self.update_agent_system_prompt_append(workspace_id, req.system_prompt_append)
            .await
            .map(Into::into)
            .map_err(workspace_store_error)
    }

    pub async fn subagent_system_prompt_config_for_route(
        &self,
        params: WorkspacePromptConfigRouteParams,
    ) -> Result<SubagentSystemPromptConfigRouteResponse, WorkspaceRouteError> {
        let workspace_id = parse_workspace_route_id(&params.workspace_id)?;
        self.load_subagent_system_prompt_append(workspace_id)
            .await
            .map(Into::into)
            .map_err(workspace_store_error)
    }

    pub async fn update_subagent_system_prompt_config_for_route(
        &self,
        params: WorkspacePromptConfigRouteParams,
        req: UpdateSubagentSystemPromptConfigRouteRequest,
    ) -> Result<SubagentSystemPromptConfigRouteResponse, WorkspaceRouteError> {
        let workspace_id = parse_workspace_route_id(&params.workspace_id)?;
        self.update_subagent_system_prompt_append(workspace_id, req.system_prompt_append)
            .await
            .map(Into::into)
            .map_err(workspace_store_error)
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

    fn sandbox_runtime_available_for_execution_config(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.shared_vm_container_runtime_available()
        }
        #[cfg(not(target_os = "macos"))]
        {
            true
        }
    }
}
