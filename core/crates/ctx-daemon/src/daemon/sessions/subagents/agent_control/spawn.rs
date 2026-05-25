use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ctx_core::ids::{SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Message, Session, SubagentInvocationChild, VcsKind, Workspace, Worktree,
};
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::provider_launch::status::provider_status_for_target;
use ctx_provider_runtime::provider_usability::{
    provider_status_is_usable, provider_status_unusable_reason,
};
use ctx_provider_runtime::ProviderRuntime;
use ctx_session_tools::model_resolution::ModelCatalog;
use ctx_settings_model::ExecutionSettings;
use ctx_store::Store;
use ctx_subagent_service::SubagentWorktreeSelection;

use super::super::errors::ApiResult;
use super::super::{
    api_error, build_spawned_agent_detail, dispatch_subagent_prompt,
    emit_subagent_invocation_notice, finalize_subagent_invocation, init_subagents,
    persist_subagent_prompt, run_subagent_child, AgentInitItem, AgentInitReq,
    PersistedSubagentPrompt, SpawnAgentReq, SpawnAgentResp, SubagentErrorKind,
};
use crate::daemon::DaemonState;

#[derive(Clone)]
pub(in crate::daemon) struct SubagentSpawnHost {
    daemon_state: Arc<DaemonState>,
    global_store: Store,
    providers: Arc<ProviderRuntime>,
    data_root: PathBuf,
}

impl SubagentSpawnHost {
    pub(in crate::daemon) fn new(
        daemon_state: Arc<DaemonState>,
        global_store: Store,
        providers: Arc<ProviderRuntime>,
        data_root: PathBuf,
    ) -> Self {
        Self {
            daemon_state,
            global_store,
            providers,
            data_root,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) async fn max_subagents_per_call(&self) -> ApiResult<usize> {
        let settings = ctx_settings_service::load_settings(&self.global_store)
            .await
            .map_err(super::super::internal_api_error)?;
        Ok(ctx_subagent_service::resolve_max_subagents_per_call(
            settings.subagents.as_ref().and_then(|s| s.max_per_call),
        ))
    }

    pub(in crate::daemon) async fn load_parent_session(
        &self,
        parent_id: SessionId,
    ) -> ApiResult<(Store, Session)> {
        super::super::errors::load_parent_session(self.daemon_state.as_ref(), parent_id).await
    }

    pub(in crate::daemon) async fn task_session_creation_lock(
        &self,
        task_id: TaskId,
    ) -> Arc<tokio::sync::Mutex<()>> {
        self.daemon_state.task_session_creation_lock(task_id).await
    }

    pub(in crate::daemon) async fn store_for_session(
        &self,
        session_id: SessionId,
    ) -> ApiResult<Store> {
        super::super::errors::store_for_session(self.daemon_state.as_ref(), session_id).await
    }

    pub(in crate::daemon) async fn load_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> ApiResult<Workspace> {
        self.global_store
            .get_workspace(workspace_id)
            .await
            .map_err(super::super::internal_api_error)?
            .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "workspace not found"))
    }

    pub(in crate::daemon) async fn resolve_existing_worktree_execution(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree_id: WorktreeId,
    ) -> ApiResult<crate::daemon::workspaces::ResolvedExistingWorktreeExecution> {
        crate::daemon::workspaces::resolve_existing_worktree_execution(
            &self.daemon_state,
            store,
            workspace,
            worktree_id,
        )
        .await
        .map_err(super::super::internal_api_error)
    }

    pub(in crate::daemon) async fn load_requested_model_catalogs(
        &self,
        workspace: &Workspace,
        provider_ids: &HashSet<String>,
        execution_environment: ExecutionEnvironment,
    ) -> ApiResult<HashMap<String, Option<ModelCatalog>>> {
        super::super::providers::load_requested_model_catalogs(
            self,
            workspace,
            provider_ids,
            execution_environment,
        )
        .await
    }

    pub(in crate::daemon) async fn effective_install_target_for_environment(
        &self,
        workspace_id: WorkspaceId,
        execution_environment: ExecutionEnvironment,
    ) -> anyhow::Result<InstallTarget> {
        crate::daemon::execution_effective::effective_install_target_for_environment(
            self.daemon_state.as_ref(),
            workspace_id,
            execution_environment,
        )
        .await
    }

    pub(in crate::daemon) async fn load_provider_matrix(
        &self,
    ) -> ctx_provider_matrix::ProviderMatrix {
        self.providers.load_provider_matrix(&self.data_root).await
    }

    pub(in crate::daemon) async fn known_harness_provider_ids(
        &self,
        matrix: &ctx_provider_matrix::ProviderMatrix,
    ) -> HashSet<String> {
        self.providers.known_harness_provider_ids(matrix).await
    }

    pub(in crate::daemon) async fn provider_unusable_reason_for_target(
        &self,
        managed: &ctx_managed_installs::AgentServerConfigFile,
        matrix: &ctx_provider_matrix::ProviderMatrix,
        provider_id: &str,
        install_target: InstallTarget,
    ) -> Option<String> {
        let status = provider_status_for_target(
            self.daemon_state.as_ref(),
            managed,
            matrix,
            provider_id,
            install_target,
        )
        .await;
        (!provider_status_is_usable(&status)).then(|| {
            provider_status_unusable_reason(&status)
                .unwrap_or_else(|| "provider not ready for use".to_string())
        })
    }

    pub(in crate::daemon) async fn load_provider_model_catalog_for_execution_environment(
        &self,
        workspace: &Workspace,
        provider_id: &str,
        execution_environment: ExecutionEnvironment,
    ) -> Result<Option<ModelCatalog>, String> {
        crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment(
            self.daemon_state.as_ref(),
            workspace,
            provider_id,
            execution_environment,
        )
        .await
    }

    pub(in crate::daemon) async fn plan_subagent_worktree_creation(
        &self,
        parent_worktree: &Worktree,
        selection: SubagentWorktreeSelection,
    ) -> ApiResult<Option<(VcsKind, String)>> {
        super::super::worktrees::plan_subagent_worktree_creation(
            &self.daemon_state,
            parent_worktree,
            selection,
        )
        .await
    }

    pub(in crate::daemon) async fn create_subagent_worktree(
        &self,
        store: &Store,
        workspace: &Workspace,
        task_id: TaskId,
        base_commit_sha: &str,
        vcs_kind: VcsKind,
        effective: &ExecutionSettings,
    ) -> ApiResult<Worktree> {
        super::super::worktrees::create_subagent_worktree(
            &self.daemon_state,
            store,
            workspace,
            task_id,
            base_commit_sha,
            vcs_kind,
            effective,
        )
        .await
    }

    pub(in crate::daemon) async fn upsert_workspace_session_index(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        self.global_store
            .upsert_workspace_session_index(session_id, workspace_id)
            .await
    }

    pub(in crate::daemon) async fn persist_subagent_prompt(
        &self,
        session: &Session,
        prompt: String,
    ) -> ApiResult<PersistedSubagentPrompt> {
        persist_subagent_prompt(&self.daemon_state, session, prompt).await
    }

    pub(in crate::daemon) async fn emit_subagent_invocation_notice(
        &self,
        parent_session_id: SessionId,
        parent_turn_id: Option<TurnId>,
        payload: serde_json::Value,
    ) -> ApiResult<()> {
        emit_subagent_invocation_notice(
            &self.daemon_state,
            parent_session_id,
            parent_turn_id,
            payload,
        )
        .await
    }

    pub(in crate::daemon) async fn dispatch_subagent_prompt(
        &self,
        session: &Session,
        saved: &Message,
    ) {
        dispatch_subagent_prompt(&self.daemon_state, session, saved).await;
    }

    pub(in crate::daemon) async fn emit_compat_payload_reject_counter(
        &self,
        surface: &str,
        issue: &str,
        extra_label: Option<(&str, &str)>,
    ) {
        self.daemon_state
            .emit_compat_payload_reject_counter(surface, issue, extra_label)
            .await;
    }

    pub(in crate::daemon) async fn emit_product_fallback_applied_counter(
        &self,
        surface: &str,
        fallback: &str,
        extra_label: Option<(&str, &str)>,
    ) {
        self.daemon_state
            .emit_product_fallback_applied_counter(surface, fallback, extra_label)
            .await;
    }

    pub(in crate::daemon) fn spawn_subagent_completion_task(
        &self,
        child: SubagentInvocationChild,
        invocation_id: String,
        tool_call_id: String,
        parent_id: SessionId,
        parent_turn_id: Option<TurnId>,
        parent_worktree_id: WorktreeId,
    ) {
        let state_weak = Arc::downgrade(&self.daemon_state);
        tokio::spawn(async move {
            if let Err(error) = run_subagent_child(&state_weak, child, parent_worktree_id).await {
                tracing::warn!(error = %error, "subagent execution failed");
            }
            if let Some(state) = state_weak.upgrade() {
                if let Err(error) = finalize_subagent_invocation(
                    &state,
                    &invocation_id,
                    &tool_call_id,
                    parent_id,
                    parent_turn_id,
                )
                .await
                {
                    tracing::warn!(error = %error, "failed to finalize subagent invocation");
                }
            }
        });
    }

    pub(in crate::daemon) async fn spawn_agent(
        self: &Arc<Self>,
        parent_id: SessionId,
        req: SpawnAgentReq,
    ) -> ApiResult<SpawnAgentResp> {
        spawn_agent_with_host(Arc::clone(self), parent_id, req).await
    }
}

async fn spawn_agent_with_host(
    host: Arc<SubagentSpawnHost>,
    parent_id: SessionId,
    req: SpawnAgentReq,
) -> ApiResult<SpawnAgentResp> {
    let task_label = req.task_label.trim().to_string();
    if task_label.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "task_label is required",
        ));
    }
    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "prompt is required",
        ));
    }

    let spawned_children = init_subagents(
        host,
        parent_id,
        AgentInitReq {
            tool_call_id: req.tool_call_id,
            response_mode: None,
            worktree: req.worktree,
            agents: vec![AgentInitItem {
                prompt,
                label: Some(task_label.clone()),
                harness: req.harness,
                model: req.model,
                reasoning_effort: req.reasoning_effort,
            }],
        },
    )
    .await?;
    let spawned = spawned_children
        .into_iter()
        .next()
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "spawned agent not found"))?;
    Ok(SpawnAgentResp {
        agent: build_spawned_agent_detail(&spawned),
    })
}
