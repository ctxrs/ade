use std::sync::Arc;

use anyhow::Result;
use ctx_core::ids::{SessionId, TaskId, TurnId};
use ctx_core::models::{Session, SessionEvent, SessionSummary, SubagentInvocation, Task, Worktree};
use ctx_observability::ops_events::OpsEvent;
use ctx_observability::telemetry::TelemetryEvent;
use ctx_session_title_service::title_generation::TitleGenerationOutcome;
use ctx_session_tools::model_resolution::compose_model_id;
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::Store;
use tokio::sync::{mpsc, Mutex};

use super::{subagents, title_generation, title_generation::schedule_session_title_generation};
use crate::daemon::handle::{SessionTitleModelModeHandle, SessionsHandle};
use crate::daemon::{require_scoped_mcp_session_context, ScopedMcpSessionAccessError};

#[derive(Debug)]
pub enum GenerateSessionTitleError {
    NotFound,
    PromptRequired,
    Skipped,
    Internal(anyhow::Error),
}

impl SessionTitleModelModeHandle {
    pub async fn generate_session_title_for_request(
        &self,
        session_id: SessionId,
        prompt: Option<String>,
        force: Option<bool>,
    ) -> Result<Session, GenerateSessionTitleError> {
        let store = self
            .session_store_or_none(session_id)
            .await
            .map_err(GenerateSessionTitleError::Internal)?
            .ok_or(GenerateSessionTitleError::NotFound)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(GenerateSessionTitleError::Internal)?
            .ok_or(GenerateSessionTitleError::NotFound)?;

        let prompt = if let Some(prompt) = prompt
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            prompt
        } else {
            store
                .get_first_user_message_content(session_id)
                .await
                .map_err(GenerateSessionTitleError::Internal)?
                .filter(|value| !value.trim().is_empty())
                .ok_or(GenerateSessionTitleError::PromptRequired)?
        };

        let force = force.unwrap_or(true);
        let cfg = self.configured_title_generation_settings().await;
        self.maybe_generate_session_title(session, prompt, force, cfg)
            .await
            .map_err(GenerateSessionTitleError::Internal)?
            .ok_or(GenerateSessionTitleError::Skipped)?;

        store
            .get_session(session_id)
            .await
            .map_err(GenerateSessionTitleError::Internal)?
            .ok_or(GenerateSessionTitleError::NotFound)
    }

    pub async fn configured_title_generation_settings(
        &self,
    ) -> Option<ctx_settings_model::TitleGenerationSettings> {
        title_generation::configured_title_generation_settings_for_store(self.global_store()).await
    }

    pub async fn maybe_generate_session_title(
        &self,
        session: Session,
        prompt: String,
        force: bool,
        cfg: Option<ctx_settings_model::TitleGenerationSettings>,
    ) -> anyhow::Result<Option<TitleGenerationOutcome>> {
        title_generation::maybe_generate_session_title_with_handle(
            self, session, prompt, force, cfg,
        )
        .await
    }

    pub async fn schedule_session_title_generation(
        &self,
        session: Session,
        prompt: String,
        force: bool,
    ) -> bool {
        let cfg = self.configured_title_generation_settings().await;
        if cfg.is_some() {
            let handle = self.clone();
            tokio::spawn(async move {
                let _ = handle
                    .maybe_generate_session_title(session, prompt, force, cfg)
                    .await;
            });
            true
        } else {
            let _ = self
                .maybe_generate_session_title(session, prompt, force, cfg)
                .await;
            false
        }
    }
}

impl SessionsHandle {
    pub async fn list_session_subagents_for_request(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Vec<SessionSummary>>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        store.list_subagent_sessions(session.id).await.map(Some)
    }

    pub async fn list_session_subagent_invocations_for_request(
        &self,
        session_id: SessionId,
        turn_id: Option<TurnId>,
    ) -> Result<Option<Vec<SubagentInvocation>>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        store
            .list_subagent_invocations_for_session(session.id, turn_id)
            .await
            .map(Some)
    }

    pub async fn get_session_subagent_invocation_for_request(
        &self,
        session_id: SessionId,
        invocation_id: &str,
    ) -> Result<Option<SubagentInvocation>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(invocation) = store.get_subagent_invocation(invocation_id).await? else {
            return Ok(None);
        };
        if invocation.parent_session_id != session_id {
            return Ok(None);
        }
        Ok(Some(invocation))
    }

    pub async fn emit_session_started_observability_for_task(
        &self,
        session: &Session,
        task: &Task,
    ) {
        let worktree = match self.store_for_session(session.id).await {
            Ok(store) => store.get_worktree(session.worktree_id).await.ok().flatten(),
            Err(_) => None,
        };
        let session_root_kind = session_root_kind_for_worktree(worktree.as_ref()).to_string();
        self.emit_session_started_signals(session, &session_root_kind)
            .await;
        if let Err(error) = self.emit_workspace_task_upsert(task.id).await {
            tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {error:?}");
        }
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        self.state.remember_session_meta(session).await;
    }

    pub async fn publish_event(&self, event: SessionEvent) {
        self.state.publish_event(event).await;
    }

    pub async fn refresh_session_head_cache(&self, session_id: SessionId) {
        self.state.refresh_session_head_cache(session_id).await;
    }

    pub async fn task_session_creation_lock(&self, task_id: TaskId) -> Arc<Mutex<()>> {
        self.state.task_session_creation_lock(task_id).await
    }

    pub async fn is_session_running(&self, session_id: SessionId) -> bool {
        self.state.is_session_running(session_id).await
    }

    pub async fn session_order_seq_state(
        &self,
        store: &Store,
        session_id: SessionId,
    ) -> Arc<Mutex<OrderSeqState>> {
        self.state.session_order_seq_state(store, session_id).await
    }

    pub async fn ensure_scheduler(
        &self,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        self.state.ensure_scheduler(session).await
    }

    pub async fn configured_title_generation_settings(
        &self,
    ) -> Option<ctx_settings_model::TitleGenerationSettings> {
        title_generation::configured_title_generation_settings(self.state.as_ref()).await
    }

    pub async fn maybe_generate_session_title(
        &self,
        session: Session,
        prompt: String,
        force: bool,
        cfg: Option<ctx_settings_model::TitleGenerationSettings>,
    ) -> anyhow::Result<Option<TitleGenerationOutcome>> {
        title_generation::maybe_generate_session_title(
            Arc::clone(&self.state),
            session,
            prompt,
            force,
            cfg,
        )
        .await
    }

    pub async fn schedule_session_title_generation(
        &self,
        session: Session,
        prompt: String,
        force: bool,
    ) -> bool {
        schedule_session_title_generation(Arc::clone(&self.state), session, prompt, force).await
    }

    pub async fn emit_session_started_signals(&self, session: &Session, session_root_kind: &str) {
        self.state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::session_started(
                session.provider_id.clone(),
                compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
                Some(session.execution_environment.as_str().to_string()),
                Some(session_root_kind.to_string()),
            ))
            .await;
        let mut ops_event = OpsEvent::new("info", "session_started");
        ops_event.session_id = Some(session.id.0.to_string());
        ops_event.worktree_id = Some(session.worktree_id.0.to_string());
        ops_event.provider_id = Some(session.provider_id.clone());
        ops_event.meta = Some(serde_json::json!({
            "model_id": compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
            "reasoning_effort": session.reasoning_effort.clone(),
            "execution_environment": session.execution_environment.as_str(),
            "session_root_kind": session_root_kind,
            "parent_session_id": session.parent_session_id.map(|id| id.0.to_string()),
            "relationship": session.relationship.clone(),
        }));
        self.state.telemetry.ops_events.emit(ops_event);
    }

    pub async fn emit_compat_payload_reject_counter(
        &self,
        surface: &str,
        issue: &str,
        extra_label: Option<(&str, &str)>,
    ) {
        self.state
            .emit_compat_payload_reject_counter(surface, issue, extra_label)
            .await;
    }

    pub async fn require_scoped_mcp_session_context(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        session_id: SessionId,
    ) -> Result<(), ScopedMcpSessionAccessError> {
        require_scoped_mcp_session_context(&self.state, mcp_auth, session_id).await
    }

    pub async fn spawn_agent(
        &self,
        parent_id: SessionId,
        req: subagents::SpawnAgentReq,
    ) -> Result<subagents::SpawnAgentResp, subagents::SubagentError> {
        subagents::spawn_agent(Arc::clone(&self.state), parent_id, req).await
    }

    pub async fn send_input(
        &self,
        parent_id: SessionId,
        req: subagents::SendInputReq,
    ) -> Result<subagents::SendInputResp, subagents::SubagentError> {
        subagents::send_input(Arc::clone(&self.state), parent_id, req).await
    }

    pub async fn archive_agent(
        &self,
        parent_id: SessionId,
        req: subagents::ArchiveAgentReq,
    ) -> Result<subagents::ArchiveAgentResp, subagents::SubagentError> {
        subagents::archive_agent(Arc::clone(&self.state), parent_id, req).await
    }

    pub async fn list_agents(
        &self,
        parent_id: SessionId,
    ) -> Result<Vec<subagents::AgentSummary>, subagents::SubagentError> {
        subagents::list_agents(Arc::clone(&self.state), parent_id).await
    }

    pub async fn get_agent(
        &self,
        parent_id: SessionId,
        req: subagents::GetAgentReq,
    ) -> Result<subagents::GetAgentResp, subagents::SubagentError> {
        subagents::get_agent(Arc::clone(&self.state), parent_id, req).await
    }

    pub async fn interrupt_agent(
        &self,
        parent_id: SessionId,
        req: subagents::InterruptAgentReq,
    ) -> Result<subagents::InterruptAgentResp, subagents::SubagentError> {
        subagents::interrupt_agent(Arc::clone(&self.state), parent_id, req).await
    }

    pub async fn wait_agent(
        &self,
        parent_id: SessionId,
        req: subagents::WaitAgentReq,
    ) -> Result<subagents::WaitAgentResp, subagents::SubagentError> {
        subagents::wait_agent(Arc::clone(&self.state), parent_id, req).await
    }
}

fn session_root_kind_for_worktree(worktree: Option<&Worktree>) -> &'static str {
    match worktree.and_then(|worktree| worktree.git_branch.as_ref()) {
        Some(_) => "worktree",
        None => "workspace_root",
    }
}
