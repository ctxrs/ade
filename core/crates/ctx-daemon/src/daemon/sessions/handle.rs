use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Artifact, ExecutionEnvironment, Message, MessageAttachment, MessageDelivery, MessageRole,
    Session, SessionEvent, SessionEventType, SessionEventsPage, SessionGitStatusSummary,
    SessionHeadSnapshot, SessionHistoryPage, SessionSnapshot, SessionState, SessionSummary,
    SessionTurn, SessionTurnStatus, SessionTurnTool, SubagentInvocation, Task, Workspace, Worktree,
    WorktreeVcsSnapshot,
};
use ctx_observability::ops_events::OpsEvent;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_observability::telemetry::TelemetryEvent;
use ctx_provider_install::install_state::InstallTarget;
use ctx_providers::adapters::ProviderAdapter;
use ctx_session_service::message_delivery::{
    build_user_message_turn, delivery_matches,
    resolve_message_delivery as resolve_message_delivery_policy, MessageDeliveryResolutionError,
};
use ctx_session_tools::model_resolution::{compose_model_id, ModelCatalog};
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::{is_unique_constraint_violation, Store};
use ctx_workspace_services::worktree_vcs::{
    resolve_worktree_diff_base_from_source, GitStatusSnapshot, WorktreeDiffBaseResolution,
    WorktreeVcsCommitLookupSource, WorktreeVcsDiffBaseQuery, WorktreeVcsDiffSummaryCounts,
};
use sha2::Digest;
use tokio::sync::{mpsc, Mutex};

use super::{
    ask_user, auth, command_dispatch, model_catalog, subagents, title_generation,
    title_generation::{schedule_session_title_generation, TitleGenerationOutcome},
};
use crate::daemon::git_status::{
    load_git_status_snapshot, worktree_has_vcs_repo, HttpWorktreeVcsSource,
};
use crate::daemon::handle::SessionsHandle;
use crate::daemon::maintenance::post_message_update_drain_reason;
use crate::daemon::workspaces::{
    complete_files_for_session, diff_worktree_for_session, diff_worktree_summary_for_session,
    resolve_existing_worktree_execution, update_workspace_provider_preferred_model_id,
    FileCompletionsError, ResolvedExistingWorktreeExecution,
};
use crate::daemon::{
    require_scoped_mcp_session_context, ScopedMcpSessionAccessError, SessionStoreAccessError,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionImageBlobStoreError {
    PayloadTooLarge,
    UnsupportedMediaType,
    Internal,
}

pub struct WorkspaceStoreContext {
    pub workspace: Workspace,
    pub store: Store,
}

pub struct DemoSeedTranscript {
    pub session_title: Option<String>,
    pub task_title: Option<String>,
    pub append: bool,
    pub refresh: bool,
    pub materialize_tail_turns: Option<usize>,
    pub turns: Vec<DemoSeedTranscriptTurn>,
}

pub struct DemoSeedTranscriptTurn {
    pub user: String,
    pub assistant: String,
    pub context_window: Option<serde_json::Value>,
}

pub struct DemoSeedTranscriptResult {
    pub seeded_turns: usize,
    pub seeded_messages: usize,
    pub seeded_events: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DemoSeedTranscriptError {
    SessionNotFound,
    SessionAlreadyHasMessages,
    StoreUnavailable,
    InspectMessages,
    UpdateSessionTitle,
    UpdateTaskTitle,
    ReloadSession,
    InsertUserMessage,
    InsertAssistantMessage,
    InsertSessionTurn,
    AppendUserEvent,
    AppendTurnStartedEvent,
    AppendAssistantEvent,
    AppendDoneEvent,
    AppendTurnFinishedEvent,
}

#[derive(Debug)]
pub enum SetSessionModeError {
    NotFound,
    BadRequest,
    Internal,
}

#[derive(Debug)]
pub enum GenerateSessionTitleError {
    NotFound,
    PromptRequired,
    Skipped,
    Internal(anyhow::Error),
}

pub struct PostUserMessageInput {
    pub message_id: MessageId,
    pub turn_id: TurnId,
    pub client_supplied_ids: bool,
    pub content: String,
    pub requested_delivery: Option<MessageDelivery>,
    pub attachments: Vec<MessageAttachment>,
    pub queued_messages_enabled: bool,
    pub run_id_header: Option<String>,
}

#[derive(Debug)]
pub enum PostUserMessageError {
    BadRequest(String),
    Conflict(String),
    NotFound(String),
    ServiceUnavailable(String),
    Internal(String),
}

#[derive(Debug)]
pub enum SessionModelTargetLoadError {
    NotFound(&'static str),
    ExecutionSettings(anyhow::Error),
    Internal(anyhow::Error),
}

impl SessionsHandle {
    pub async fn get_blob(
        &self,
        id: &str,
    ) -> Result<
        Option<(
            String,
            String,
            i64,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        )>,
    > {
        self.state.global_store().get_blob(id).await
    }

    pub async fn get_workspace(&self, workspace_id: WorkspaceId) -> Result<Option<Workspace>> {
        self.state.global_store().get_workspace(workspace_id).await
    }

    pub async fn get_workspace_id_for_task(&self, task_id: TaskId) -> Result<Option<WorkspaceId>> {
        self.state
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await
    }

    pub async fn get_workspace_id_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<WorkspaceId>> {
        self.state
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
    }

    pub async fn get_session_for_artifacts(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Session>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        store.get_session(session_id).await
    }

    pub async fn get_session_worktree(&self, session: &Session) -> Result<Option<Worktree>> {
        let store = self.store_for_session(session.id).await?;
        store.get_worktree(session.worktree_id).await
    }

    pub async fn get_session_artifact_for_download(
        &self,
        session_id: SessionId,
        artifact_id: ctx_core::ids::ArtifactId,
    ) -> Result<Option<(Session, Artifact)>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        let Some(artifact) = store.get_artifact(artifact_id).await? else {
            return Ok(None);
        };
        if artifact.session_id != session.id {
            return Ok(None);
        }
        Ok(Some((session, artifact)))
    }

    pub async fn list_session_artifacts_for_route(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(Session, Vec<Artifact>)>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        let artifacts = store.list_session_artifacts(session.id).await?;
        Ok(Some((session, artifacts)))
    }

    pub async fn replace_session_artifacts_and_publish(
        &self,
        session: &Session,
        artifacts: &[Artifact],
    ) -> Result<()> {
        let store = self.store_for_session(session.id).await?;
        store
            .replace_session_artifacts(session.id, artifacts)
            .await?;
        let event = store
            .append_session_event(
                session.id,
                None,
                None,
                SessionEventType::ArtifactsSet,
                serde_json::json!({ "artifacts": artifacts }),
            )
            .await?;
        self.publish_event(event).await;
        Ok(())
    }

    pub async fn seed_demo_transcript(
        &self,
        session_id: SessionId,
        seed: DemoSeedTranscript,
    ) -> Result<DemoSeedTranscriptResult, DemoSeedTranscriptError> {
        let store = self
            .store_for_session(session_id)
            .await
            .map_err(|_| DemoSeedTranscriptError::SessionNotFound)?;
        let mut session = store
            .get_session(session_id)
            .await
            .map_err(|_| DemoSeedTranscriptError::StoreUnavailable)?
            .ok_or(DemoSeedTranscriptError::SessionNotFound)?;

        if !seed.append
            && !store
                .list_messages_for_session(session_id)
                .await
                .map_err(|_| DemoSeedTranscriptError::InspectMessages)?
                .is_empty()
        {
            return Err(DemoSeedTranscriptError::SessionAlreadyHasMessages);
        }

        if let Some(session_title) = seed
            .session_title
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            store
                .update_session_title(session_id, session_title.to_string())
                .await
                .map_err(|_| DemoSeedTranscriptError::UpdateSessionTitle)?;
        }

        if let Some(task_title) = seed
            .task_title
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            store
                .update_task_title(session.task_id, task_title.to_string())
                .await
                .map_err(|_| DemoSeedTranscriptError::UpdateTaskTitle)?;
        }

        session = store
            .get_session(session_id)
            .await
            .map_err(|_| DemoSeedTranscriptError::ReloadSession)?
            .ok_or(DemoSeedTranscriptError::SessionNotFound)?;
        self.remember_session_meta(&session).await;

        let mut seeded_messages = 0usize;
        let mut seeded_events = 0usize;
        let base_time = Utc::now() - ChronoDuration::minutes(seed.turns.len() as i64);
        let materialize_from_index = seed
            .materialize_tail_turns
            .map(|tail| seed.turns.len().saturating_sub(tail));

        for (index, turn) in seed.turns.iter().enumerate() {
            let materialize_turn = materialize_from_index
                .map(|from_index| index >= from_index)
                .unwrap_or(true);
            let seeded = DemoSeededTurn::new(session_id, session.task_id, index, base_time, turn);
            let counts = self
                .seed_demo_transcript_turn(&store, &seeded, turn, materialize_turn)
                .await?;
            seeded_messages += counts.seeded_messages;
            seeded_events += counts.seeded_events;
        }

        if seed.refresh {
            self.refresh_session_head_cache(session_id).await;
            if let Err(error) = self.emit_workspace_task_upsert(session.task_id).await {
                tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed after demo transcript seed: {error:?}");
            }
        }

        Ok(DemoSeedTranscriptResult {
            seeded_turns: seed.turns.len(),
            seeded_messages,
            seeded_events,
        })
    }

    pub async fn load_session_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Result<Option<SessionSnapshot>> {
        let Some(store) = self
            .session_store_allow_archived_or_none(session_id)
            .await?
        else {
            return Ok(None);
        };
        store
            .get_session_snapshot(session_id, limit, include_events)
            .await
    }

    pub async fn list_session_events_page(
        &self,
        session_id: SessionId,
        after_seq: Option<i64>,
        limit: u32,
        tail: Option<u32>,
        include_transient: bool,
    ) -> Result<Option<SessionEventsPage>> {
        const MAX_LIMIT: u32 = 1000;

        let Some(store) = self
            .session_store_allow_archived_or_none(session_id)
            .await?
        else {
            return Ok(None);
        };

        let (events, has_more, next_cursor) = if let Some(tail) = tail {
            let tail = tail.clamp(1, MAX_LIMIT);
            let mut rows = store
                .list_session_events_tail_by_seq(session_id, tail + 1, include_transient)
                .await?;
            let has_more = rows.len() as u32 > tail;
            if has_more {
                rows = rows.split_off(rows.len().saturating_sub(tail as usize));
            }
            let next_cursor = rows.last().map(|ev| ev.seq);
            (rows, has_more, next_cursor)
        } else {
            let limit = limit.clamp(1, MAX_LIMIT);
            let mut rows = store
                .list_session_events_page_by_seq(
                    session_id,
                    after_seq,
                    Some(limit + 1),
                    include_transient,
                )
                .await?;
            let has_more = rows.len() as u32 > limit;
            if has_more {
                rows.truncate(limit as usize);
            }
            let next_cursor = rows.last().map(|ev| ev.seq);
            (rows, has_more, next_cursor)
        };

        Ok(Some(SessionEventsPage {
            session_id,
            events,
            next_cursor,
            has_more,
        }))
    }

    pub async fn load_session_history_page(
        &self,
        session_id: SessionId,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Option<SessionHistoryPage>> {
        let Some(store) = self
            .session_store_allow_archived_or_none(session_id)
            .await?
        else {
            return Ok(None);
        };
        store
            .get_session_history_page(session_id, before_seq, limit)
            .await
    }

    pub async fn list_session_turn_tools_for_request(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<Option<Vec<SessionTurnTool>>> {
        let Some(store) = self
            .session_store_allow_archived_or_none(session_id)
            .await?
        else {
            return Ok(None);
        };
        store.list_turn_tools(session_id, turn_id).await.map(Some)
    }

    pub async fn load_session_state(&self, session_id: SessionId) -> Result<Option<SessionState>> {
        let Some(store) = self
            .session_store_allow_archived_or_none(session_id)
            .await?
        else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        let mut session_state = store.get_session_state(session_id).await?;
        for artifact in session_state.artifacts.iter_mut() {
            if !self
                .session_artifact_path_is_accessible(
                    &store,
                    &session,
                    std::path::Path::new(&artifact.absolute_path),
                )
                .await?
            {
                artifact.missing = Some(true);
            }
        }
        Ok(Some(session_state))
    }

    pub async fn load_session_head_snapshot_from_store(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Result<Option<SessionHeadSnapshot>> {
        let Some(store) = self
            .session_store_allow_archived_or_none(session_id)
            .await?
        else {
            return Ok(None);
        };
        store
            .get_session_head_snapshot(session_id, limit, include_events)
            .await
    }

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

    pub async fn load_session_vcs_parts(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(Session, Worktree)>> {
        let Some(store) = self.session_store_or_none(session_id).await? else {
            return Ok(None);
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        let Some(worktree) = store.get_worktree(session.worktree_id).await? else {
            return Ok(None);
        };
        Ok(Some((session, worktree)))
    }

    pub async fn persist_session_git_status_summary(
        &self,
        session_id: SessionId,
        worktree_id: WorktreeId,
        summary: &SessionGitStatusSummary,
    ) -> Result<()> {
        let store = self.store_for_session(session_id).await?;
        store
            .upsert_session_git_status_summary(session_id, worktree_id, summary)
            .await
    }

    async fn session_store_allow_archived_or_none(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Store>> {
        match self.existing_session_store_allow_archived(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }

    async fn session_store_or_none(&self, session_id: SessionId) -> Result<Option<Store>> {
        match self.existing_session_store(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }

    async fn session_store_for_write_or_none(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Store>> {
        match self.existing_session_store_for_write(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }

    async fn seed_demo_transcript_turn(
        &self,
        store: &Store,
        seeded: &DemoSeededTurn,
        turn: &DemoSeedTranscriptTurn,
        materialize_turn: bool,
    ) -> Result<DemoSeedTranscriptResult, DemoSeedTranscriptError> {
        let mut seeded_messages = 0usize;
        let mut seeded_events = 0usize;

        if materialize_turn {
            store
                .insert_message(seeded.user_message.clone())
                .await
                .map_err(|_| DemoSeedTranscriptError::InsertUserMessage)?;
            seeded_messages += 1;
        }

        let user_event = store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::UserMessage,
                serde_json::json!({
                    "message_id": seeded.user_message.id.0,
                    "content": &seeded.user_message.content,
                    "delivery": &seeded.user_message.delivery,
                    "attachments": [],
                    "order_seq": seeded.user_order_seq,
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendUserEvent)?;
        seeded_events += 1;

        store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::TurnStarted,
                serde_json::json!({
                    "message_id": seeded.user_message.id.0,
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendTurnStartedEvent)?;
        seeded_events += 1;

        if materialize_turn {
            store
                .insert_message(seeded.assistant_message.clone())
                .await
                .map_err(|_| DemoSeedTranscriptError::InsertAssistantMessage)?;
            seeded_messages += 1;
        }

        store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::AssistantMessageInserted,
                serde_json::json!({
                    "message_id": seeded.assistant_message.id.0,
                    "content": &seeded.assistant_message.content,
                    "attachments": [],
                    "delivery": &seeded.assistant_message.delivery,
                    "order_seq": seeded.assistant_order_seq,
                    "turn_sequence": 1,
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendAssistantEvent)?;
        seeded_events += 1;

        let done_event = store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::Done,
                {
                    let mut payload = serde_json::json!({ "status": "completed" });
                    if let Some(metrics) = turn.context_window.as_ref() {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("context_window".to_string(), metrics.clone());
                        }
                    }
                    payload
                },
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendDoneEvent)?;
        seeded_events += 1;

        store
            .append_session_event(
                seeded.session_id,
                Some(seeded.run_id),
                Some(seeded.turn_id),
                SessionEventType::TurnFinished,
                serde_json::json!({
                    "message_id": seeded.user_message.id.0,
                    "status": "completed",
                }),
            )
            .await
            .map_err(|_| DemoSeedTranscriptError::AppendTurnFinishedEvent)?;
        seeded_events += 1;

        if materialize_turn {
            store
                .insert_session_turn(SessionTurn {
                    turn_id: seeded.turn_id,
                    session_id: seeded.session_id,
                    run_id: Some(seeded.run_id),
                    user_message_id: Some(seeded.user_message.id),
                    status: SessionTurnStatus::Completed,
                    start_seq: Some(user_event.seq),
                    end_seq: Some(done_event.seq),
                    started_at: seeded.user_created_at,
                    updated_at: seeded.assistant_created_at,
                    assistant_partial: None,
                    thought_partial: None,
                    metrics_json: turn.context_window.clone(),
                    failure: None,
                    tool_total: 0,
                    tool_pending: 0,
                    tool_running: 0,
                    tool_completed: 0,
                    tool_failed: 0,
                })
                .await
                .map_err(|_| DemoSeedTranscriptError::InsertSessionTurn)?;
        }

        Ok(DemoSeedTranscriptResult {
            seeded_turns: 1,
            seeded_messages,
            seeded_events,
        })
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

    pub async fn upsert_workspace_task_index(
        &self,
        task_id: TaskId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.state
            .global_store()
            .upsert_workspace_task_index(task_id, workspace_id)
            .await
    }

    pub async fn upsert_workspace_session_index(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.state
            .global_store()
            .upsert_workspace_session_index(session_id, workspace_id)
            .await
    }

    pub async fn delete_workspace_worktree_index(&self, worktree_id: WorktreeId) -> Result<()> {
        self.state
            .global_store()
            .delete_workspace_worktree_index(worktree_id)
            .await
    }

    pub async fn load_workspace_context(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceStoreContext>> {
        let Some(workspace) = self.get_workspace(workspace_id).await? else {
            return Ok(None);
        };
        let store = self.store_for_workspace(workspace_id).await?;
        Ok(Some(WorkspaceStoreContext { workspace, store }))
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store> {
        self.state.store_for_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn store_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Store> {
        self.state.store_for_session(session_id).await
    }

    pub(in crate::daemon) async fn existing_session_store_allow_archived(
        &self,
        session_id: SessionId,
    ) -> Result<Store, SessionStoreAccessError> {
        self.state
            .existing_session_store_allow_archived(session_id)
            .await
    }

    pub(in crate::daemon) async fn existing_session_store(
        &self,
        session_id: SessionId,
    ) -> Result<Store, SessionStoreAccessError> {
        self.state.existing_session_store(session_id).await
    }

    pub(in crate::daemon) async fn existing_session_store_for_write(
        &self,
        session_id: SessionId,
    ) -> Result<Store, SessionStoreAccessError> {
        self.state
            .existing_session_store_for_write(session_id)
            .await
    }

    pub async fn complete_files_for_session(
        &self,
        session_id: SessionId,
        query: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<String>, FileCompletionsError> {
        complete_files_for_session(&self.state, session_id, query, limit).await
    }

    pub async fn cancel_session(
        &self,
        session_id: SessionId,
    ) -> Result<(), command_dispatch::SessionSchedulerCommandError> {
        command_dispatch::cancel_session(&self.state, session_id).await
    }

    pub async fn interrupt_session(
        &self,
        session_id: SessionId,
        request_started: Instant,
    ) -> Result<(), command_dispatch::SessionSchedulerCommandError> {
        command_dispatch::interrupt_session(&self.state, session_id, request_started).await
    }

    pub async fn delete_queued_session_message(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<(), command_dispatch::SessionSchedulerCommandError> {
        command_dispatch::delete_queued_session_message(&self.state, session_id, message_id).await
    }

    pub async fn enqueue_user_message_for_scheduler(
        &self,
        store: &Store,
        session: Session,
        message: Message,
        run_id_header: Option<String>,
    ) {
        command_dispatch::enqueue_user_message_for_scheduler(
            &self.state,
            store,
            session,
            message,
            run_id_header,
        )
        .await;
    }

    pub async fn submit_ask_user_answer(
        &self,
        session_id: SessionId,
        submission: ask_user::SubmitAskUserAnswer,
    ) -> Result<(), ask_user::SubmitAskUserAnswerError> {
        ask_user::submit_ask_user_answer(&self.state, session_id, submission).await
    }

    pub async fn authenticate_session(
        &self,
        store: &Store,
        session: &Session,
        method_id: Option<String>,
    ) -> Result<(), auth::SessionAuthError> {
        auth::run_session_authentication(&self.state, store, session, method_id).await
    }

    pub async fn authenticate_session_for_request(
        &self,
        session_id: SessionId,
        method_id: Option<String>,
    ) -> Result<(), auth::SessionAuthError> {
        let store = self
            .existing_session_store_for_write(session_id)
            .await
            .map_err(session_store_access_auth_error)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(|_| auth::SessionAuthError::Internal("failed to load session".to_string()))?
            .ok_or(auth::SessionAuthError::NotFound("session"))?;
        self.authenticate_session(&store, &session, method_id).await
    }

    pub async fn set_session_mode_for_request(
        &self,
        session_id: SessionId,
        mode_id: String,
    ) -> Result<(), SetSessionModeError> {
        let store = self
            .existing_session_store_for_write(session_id)
            .await
            .map_err(session_store_access_mode_error)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(|_| SetSessionModeError::Internal)?
            .ok_or(SetSessionModeError::NotFound)?;
        let worktree = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| SetSessionModeError::Internal)?
            .ok_or(SetSessionModeError::NotFound)?;
        let workspace = self
            .get_workspace(session.workspace_id)
            .await
            .map_err(|_| SetSessionModeError::Internal)?
            .ok_or(SetSessionModeError::NotFound)?;
        let resolved_worktree = self
            .resolve_existing_worktree_execution(&store, &workspace, worktree.id)
            .await
            .map_err(|_| SetSessionModeError::Internal)?;
        let execution_environment = resolved_worktree.execution_environment();
        if session.execution_environment != execution_environment {
            tracing::warn!(
                session_id = %session.id.0,
                stored = session.execution_environment.as_str(),
                resolved = execution_environment.as_str(),
                "session mode update resolved a different execution_environment than persisted metadata"
            );
        }
        let install_target = self
            .effective_install_target_for_environment(worktree.workspace_id, execution_environment)
            .await
            .map_err(|err| {
                tracing::warn!(
                    workspace_id = %worktree.workspace_id.0,
                    "set_session_mode failed to load execution settings: {err:#}",
                );
                SetSessionModeError::Internal
            })?;

        let adapter = self
            .ensure_provider_adapter_for_target(&session.provider_id, install_target)
            .await
            .map_err(|_| SetSessionModeError::Internal)?;

        adapter
            .set_session_mode(session.id.0.to_string(), mode_id.clone())
            .await
            .map_err(|_| SetSessionModeError::BadRequest)?;

        let event = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Init,
                serde_json::json!({"set_mode": mode_id}),
            )
            .await
            .map_err(|_| SetSessionModeError::Internal)?;
        self.publish_event(event).await;

        Ok(())
    }

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

    pub async fn load_session_model_target_parts(
        &self,
        session_id: SessionId,
    ) -> Result<
        (Session, Workspace, ExecutionEnvironment, InstallTarget),
        SessionModelTargetLoadError,
    > {
        let store = self
            .existing_session_store_for_write(session_id)
            .await
            .map_err(session_store_access_model_target_error)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(SessionModelTargetLoadError::Internal)?
            .ok_or(SessionModelTargetLoadError::NotFound("session"))?;
        let workspace = self
            .get_workspace(session.workspace_id)
            .await
            .map_err(SessionModelTargetLoadError::Internal)?
            .ok_or(SessionModelTargetLoadError::NotFound("workspace"))?;
        let worktree = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(SessionModelTargetLoadError::Internal)?
            .ok_or(SessionModelTargetLoadError::NotFound("worktree"))?;
        let resolved_worktree = self
            .resolve_existing_worktree_execution(&store, &workspace, worktree.id)
            .await
            .map_err(SessionModelTargetLoadError::Internal)?;
        let execution_environment = resolved_worktree.execution_environment();
        if session.execution_environment != execution_environment {
            tracing::warn!(
                session_id = %session.id.0,
                stored = session.execution_environment.as_str(),
                resolved = execution_environment.as_str(),
                "session model update resolved a different execution_environment than persisted metadata"
            );
        }
        let install_target = self
            .effective_install_target_for_environment(worktree.workspace_id, execution_environment)
            .await
            .map_err(|err| {
                tracing::warn!(
                    workspace_id = %worktree.workspace_id.0,
                    "set_session_model failed to load execution settings: {err:#}",
                );
                SessionModelTargetLoadError::ExecutionSettings(err)
            })?;

        Ok((session, workspace, execution_environment, install_target))
    }

    pub async fn persist_session_model_update_for_request(
        &self,
        session_id: SessionId,
        model_id: String,
        reasoning_effort: Option<String>,
        full_model_id: String,
    ) -> Result<Option<Session>> {
        let Some(store) = self.session_store_for_write_or_none(session_id).await? else {
            return Ok(None);
        };
        store
            .update_session_model_config(session_id, model_id, reasoning_effort.clone())
            .await?;

        let Some(updated) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        self.remember_session_meta(&updated).await;

        let event = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Init,
                serde_json::json!({
                    "current_model_id": full_model_id,
                    "reasoning_effort": reasoning_effort,
                }),
            )
            .await?;
        self.publish_event(event).await;

        if let Err(error) = self
            .update_workspace_provider_preferred_model_id(
                updated.workspace_id,
                &updated.provider_id,
                Some(full_model_id),
            )
            .await
        {
            tracing::warn!(
                session_id = %updated.id.0,
                workspace_id = %updated.workspace_id.0,
                provider_id = updated.provider_id.as_str(),
                "failed to persist workspace provider model preference after session model update: {error:#}"
            );
        }

        if let Err(error) = self.emit_workspace_task_upsert(updated.task_id).await {
            tracing::warn!(
                task_id = %updated.task_id.0,
                "workspace active snapshot refresh failed after session model update: {error:?}"
            );
        }

        Ok(Some(updated))
    }

    pub async fn post_user_message_for_request(
        &self,
        session_id: SessionId,
        input: PostUserMessageInput,
    ) -> Result<Message, PostUserMessageError> {
        let store = self
            .existing_session_store_for_write(session_id)
            .await
            .map_err(post_message_store_error)?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(|_| PostUserMessageError::Internal("Failed to load session.".to_string()))?
            .ok_or_else(|| PostUserMessageError::NotFound("Session not found.".to_string()))?;
        self.remember_session_meta(&session).await;

        if let Some(reason) = self.post_message_update_drain_reason().await {
            return Err(PostUserMessageError::ServiceUnavailable(format!(
                "Daemon update is in progress; retry after the daemon restarts. ({})",
                reason
            )));
        }

        if let Some(existing) = self
            .load_matching_existing_user_message(&store, session_id, &input)
            .await?
        {
            return Ok(existing);
        }

        let persisted = self
            .persist_user_message_for_request(&store, &session, &input)
            .await?;
        self.publish_user_message_events_for_request(&store, session_id, &persisted)
            .await?;
        self.enqueue_user_message_for_scheduler(
            &store,
            session,
            persisted.saved.clone(),
            input.run_id_header,
        )
        .await;

        Ok(persisted.saved)
    }

    async fn load_matching_existing_user_message(
        &self,
        store: &Store,
        session_id: SessionId,
        input: &PostUserMessageInput,
    ) -> Result<Option<Message>, PostUserMessageError> {
        if !input.client_supplied_ids {
            return Ok(None);
        }
        let Some(existing) = store
            .get_message(input.message_id)
            .await
            .map_err(|_| PostUserMessageError::Internal("Failed to load message.".to_string()))?
        else {
            return Ok(None);
        };
        if self
            .message_matches_post_input(&existing, session_id, input)
            .await?
        {
            self.ensure_existing_message_turn_for_request(
                store,
                session_id,
                input.turn_id,
                &existing,
            )
            .await?;
            return Ok(Some(existing));
        }
        Err(PostUserMessageError::Conflict(
            "A different message already exists for that client id.".to_string(),
        ))
    }

    async fn persist_user_message_for_request(
        &self,
        store: &Store,
        session: &Session,
        input: &PostUserMessageInput,
    ) -> Result<PersistedPostUserMessage, PostUserMessageError> {
        let delivery = resolve_message_delivery_policy(
            input.requested_delivery.clone(),
            self.is_session_running(session.id).await,
            input.queued_messages_enabled,
        )
        .map_err(post_message_delivery_error)?;

        let run_id = RunId::new();
        let order_seq_state = self.session_order_seq_state(store, session.id).await;
        let order_seq = {
            let mut order_seq_state = order_seq_state.lock().await;
            order_seq_state.get_or_assign(format!("message:{}", input.message_id.0), None)
        };
        let msg = Message {
            id: input.message_id,
            session_id: session.id,
            task_id: session.task_id,
            run_id: Some(run_id),
            turn_id: Some(input.turn_id),
            turn_sequence: Some(0),
            order_seq: Some(order_seq),
            role: MessageRole::User,
            content: input.content.clone(),
            attachments: input.attachments.clone(),
            delivery,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        };
        let saved = match store.insert_message(msg).await {
            Ok(saved) => saved,
            Err(err) if input.client_supplied_ids && is_unique_constraint_violation(&err) => {
                self.load_conflicting_existing_user_message(store, session.id, input)
                    .await?
            }
            Err(_) => {
                return Err(PostUserMessageError::Internal(
                    "Failed to save message.".to_string(),
                ))
            }
        };
        Ok(PersistedPostUserMessage {
            saved,
            run_id,
            turn_id: input.turn_id,
            order_seq,
        })
    }

    async fn load_conflicting_existing_user_message(
        &self,
        store: &Store,
        session_id: SessionId,
        input: &PostUserMessageInput,
    ) -> Result<Message, PostUserMessageError> {
        let Some(existing) = store
            .get_message(input.message_id)
            .await
            .map_err(|_| PostUserMessageError::Internal("Failed to load message.".to_string()))?
        else {
            return Err(PostUserMessageError::Internal(
                "Message already existed but could not be loaded.".to_string(),
            ));
        };
        if self
            .message_matches_post_input(&existing, session_id, input)
            .await?
        {
            Ok(existing)
        } else {
            Err(PostUserMessageError::Conflict(
                "A different message already exists for that client id.".to_string(),
            ))
        }
    }

    async fn message_matches_post_input(
        &self,
        existing: &Message,
        session_id: SessionId,
        input: &PostUserMessageInput,
    ) -> Result<bool, PostUserMessageError> {
        Ok(existing.session_id == session_id
            && existing.turn_id == Some(input.turn_id)
            && matches!(existing.role, MessageRole::User)
            && existing.content == input.content
            && match input.requested_delivery.as_ref() {
                Some(requested_delivery) => {
                    delivery_matches(&existing.delivery, requested_delivery)
                }
                None => true,
            }
            && self
                .message_attachments_match(&existing.attachments, &input.attachments)
                .await?)
    }

    async fn message_attachments_match(
        &self,
        existing: &[MessageAttachment],
        requested: &[MessageAttachment],
    ) -> Result<bool, PostUserMessageError> {
        if existing.len() != requested.len() {
            return Ok(false);
        }
        let mut existing_sig = Vec::with_capacity(existing.len());
        for attachment in existing {
            existing_sig.push(self.message_attachment_signature(attachment).await?);
        }
        let mut requested_sig = Vec::with_capacity(requested.len());
        for attachment in requested {
            requested_sig.push(self.message_attachment_signature(attachment).await?);
        }
        Ok(existing_sig == requested_sig)
    }

    async fn message_attachment_signature(
        &self,
        attachment: &MessageAttachment,
    ) -> Result<MessageAttachmentSignature, PostUserMessageError> {
        match attachment {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|_| {
                        PostUserMessageError::BadRequest("Invalid image attachment.".to_string())
                    })?;
                let mut hasher = sha2::Sha256::new();
                hasher.update(&bytes);
                Ok(MessageAttachmentSignature {
                    mime_type: mime_type.clone(),
                    name: name.clone(),
                    sha256: hex::encode(hasher.finalize()),
                })
            }
            MessageAttachment::ImageRef { blob_id, name, .. } => {
                let Some((sha256, mime_type, _bytes, _stored_name, _created_at)) =
                    self.get_blob(blob_id).await.map_err(|_| {
                        PostUserMessageError::Internal(
                            "Failed to inspect image attachment.".to_string(),
                        )
                    })?
                else {
                    return Err(PostUserMessageError::BadRequest(
                        "Image attachment blob was not found.".to_string(),
                    ));
                };
                Ok(MessageAttachmentSignature {
                    mime_type,
                    name: name.clone(),
                    sha256,
                })
            }
        }
    }

    async fn publish_user_message_events_for_request(
        &self,
        store: &Store,
        session_id: SessionId,
        persisted: &PersistedPostUserMessage,
    ) -> Result<(), PostUserMessageError> {
        let saved = &persisted.saved;
        let event = store
            .append_session_event(
                session_id,
                Some(persisted.run_id),
                Some(persisted.turn_id),
                SessionEventType::UserMessage,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "content": saved.content.clone(),
                    "delivery": saved.delivery.clone(),
                    "attachments": saved.attachments,
                    "order_seq": persisted.order_seq,
                }),
            )
            .await
            .map_err(|_| {
                PostUserMessageError::Internal("Failed to append session event.".to_string())
            })?;
        let start_seq = event.seq;

        let turn = build_user_message_turn(saved, persisted.turn_id, Some(start_seq));
        self.ensure_session_turn_for_request(store, session_id, persisted.turn_id, saved, turn)
            .await?;
        self.publish_event(event).await;

        if matches!(saved.delivery, MessageDelivery::Queued) {
            self.publish_queue_events_for_request(store, session_id, persisted)
                .await?;
        }

        Ok(())
    }

    async fn ensure_existing_message_turn_for_request(
        &self,
        store: &Store,
        session_id: SessionId,
        turn_id: TurnId,
        existing: &Message,
    ) -> Result<(), PostUserMessageError> {
        let turn = build_user_message_turn(existing, turn_id, None);
        self.ensure_session_turn_for_request(store, session_id, turn_id, existing, turn)
            .await
    }

    async fn ensure_session_turn_for_request(
        &self,
        store: &Store,
        session_id: SessionId,
        turn_id: TurnId,
        message: &Message,
        turn: SessionTurn,
    ) -> Result<(), PostUserMessageError> {
        let existing_turn = store.get_session_turn_by_id(turn_id).await.map_err(|_| {
            PostUserMessageError::Internal("Failed to inspect session turn.".to_string())
        })?;
        if let Some(existing) = existing_turn {
            let matches =
                existing.session_id == session_id && existing.user_message_id == Some(message.id);
            if !matches {
                return Err(PostUserMessageError::Conflict(
                    "Turn id already belongs to another message.".to_string(),
                ));
            }
            return Ok(());
        }

        if let Err(err) = store.insert_session_turn(turn).await {
            if !is_unique_constraint_violation(&err) {
                return Err(PostUserMessageError::Internal(
                    "Failed to create session turn.".to_string(),
                ));
            }
            let existing = store.get_session_turn_by_id(turn_id).await.map_err(|_| {
                PostUserMessageError::Internal("Failed to inspect session turn.".to_string())
            })?;
            if let Some(existing) = existing {
                let matches = existing.session_id == session_id
                    && existing.user_message_id == Some(message.id);
                if !matches {
                    return Err(PostUserMessageError::Conflict(
                        "Turn id already belongs to another message.".to_string(),
                    ));
                }
            } else {
                return Err(PostUserMessageError::Internal(
                    "Session turn insert succeeded but could not be reloaded.".to_string(),
                ));
            }
        }
        Ok(())
    }

    async fn publish_queue_events_for_request(
        &self,
        store: &Store,
        session_id: SessionId,
        persisted: &PersistedPostUserMessage,
    ) -> Result<(), PostUserMessageError> {
        let saved = &persisted.saved;
        let queued = store
            .append_session_event(
                session_id,
                Some(persisted.run_id),
                Some(persisted.turn_id),
                SessionEventType::InputQueued,
                serde_json::json!({"message_id": saved.id.0}),
            )
            .await
            .map_err(|_| {
                PostUserMessageError::Internal("Failed to append queued-input event.".to_string())
            })?;
        self.publish_event(queued).await;

        let queue_position = store
            .list_queued_messages_for_session(session_id)
            .await
            .ok()
            .and_then(|messages| {
                messages
                    .iter()
                    .position(|message| message.id == saved.id)
                    .map(|idx| idx as i64)
            });

        let queue_added = store
            .append_session_event(
                session_id,
                Some(persisted.run_id),
                Some(persisted.turn_id),
                SessionEventType::MessageQueueAdded,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(|_| {
                PostUserMessageError::Internal("Failed to append queue event.".to_string())
            })?;
        self.publish_event(queue_added).await;

        let turn_queued = store
            .append_session_event(
                session_id,
                Some(persisted.run_id),
                Some(persisted.turn_id),
                SessionEventType::TurnQueued,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(|_| {
                PostUserMessageError::Internal("Failed to append queued turn event.".to_string())
            })?;
        self.publish_event(turn_queued).await;

        Ok(())
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

    pub fn session_tool_output_spool_dir(&self, session_id: SessionId) -> PathBuf {
        self.state
            .core
            .tool_output_spool_dir
            .join(session_id.0.to_string())
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

    pub async fn post_message_update_drain_reason(&self) -> Option<String> {
        post_message_update_drain_reason(self.state.as_ref()).await
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

    pub async fn resolve_existing_worktree_execution(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<ResolvedExistingWorktreeExecution> {
        resolve_existing_worktree_execution(&self.state, store, workspace, worktree_id).await
    }

    pub async fn effective_install_target_for_environment(
        &self,
        workspace_id: WorkspaceId,
        execution_environment: ExecutionEnvironment,
    ) -> anyhow::Result<InstallTarget> {
        crate::daemon::execution_effective::effective_install_target_for_environment(
            self.state.as_ref(),
            workspace_id,
            execution_environment,
        )
        .await
    }

    pub async fn ensure_provider_adapter_for_target(
        &self,
        provider_id: &str,
        install_target: InstallTarget,
    ) -> anyhow::Result<Arc<dyn ProviderAdapter>> {
        ctx_provider_runtime::provider_launch::resolver::ensure_provider_adapter_for_target(
            self.state.as_ref(),
            provider_id,
            install_target,
        )
        .await
    }

    pub async fn load_provider_model_catalog_for_execution_environment(
        &self,
        workspace: &Workspace,
        provider_id: &str,
        execution_environment: ExecutionEnvironment,
    ) -> Result<Option<ModelCatalog>, String> {
        model_catalog::load_provider_model_catalog_for_execution_environment(
            &self.state,
            workspace,
            provider_id,
            execution_environment,
        )
        .await
    }

    pub async fn update_workspace_provider_preferred_model_id(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
        preferred_model_id: Option<String>,
    ) -> anyhow::Result<()> {
        update_workspace_provider_preferred_model_id(
            &self.state,
            workspace_id,
            provider_id,
            preferred_model_id,
        )
        .await
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> anyhow::Result<()> {
        self.state.emit_workspace_task_upsert(task_id).await
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

    pub async fn workspace_id_for_session(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<WorkspaceId>> {
        self.state
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
    }

    pub async fn is_workspace_deleting(&self, workspace_id: WorkspaceId) -> bool {
        self.state
            .core
            .stores
            .is_workspace_deleting(workspace_id)
            .await
    }

    pub async fn cached_session_head_for_request(
        &self,
        session_id: SessionId,
        include_events: bool,
        limit: u32,
        min_event_seq: Option<i64>,
    ) -> Option<SessionHeadSnapshot> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .get_cached_session_head_for_request(session_id, include_events, limit, min_event_seq)
            .await
    }

    pub async fn update_session_head_cache(&self, head: SessionHeadSnapshot, include_events: bool) {
        if include_events {
            self.state
                .workspaces
                .workspace_active_snapshot
                .update_session_head(head)
                .await;
        } else {
            self.state
                .workspaces
                .workspace_active_snapshot
                .update_compact_session_head(head)
                .await;
        }
    }

    pub async fn emit_cache_miss(&self, cache: &str) {
        self.state.emit_cache_miss(cache).await;
    }

    pub async fn emit_cache_rehydrate(&self, cache: &str, ok: bool) {
        self.state.emit_cache_rehydrate(cache, ok).await;
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

    pub fn record_session_head_recovery_metrics(
        &self,
        source: &'static str,
        result: &'static str,
        elapsed: Duration,
        limit: u32,
        include_events: bool,
        head: Option<&SessionHeadSnapshot>,
    ) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("surface".to_string(), "session_head_recovery".to_string());
        labels.insert("recovery_source".to_string(), source.to_string());
        labels.insert("result".to_string(), result.to_string());
        labels.insert(
            "include_events".to_string(),
            if include_events { "true" } else { "false" }.to_string(),
        );
        labels.insert(
            "limit_bucket".to_string(),
            session_head_limit_bucket(limit).to_string(),
        );

        let response_bytes = head
            .map(|value| value.head_window.bytes.max(0) as f64)
            .unwrap_or(0.0);
        let metrics = [
            (
                "workbench.session_head_recovery_ms",
                "ms",
                elapsed.as_millis() as f64,
            ),
            (
                "workbench.session_head_recovery_response_bytes",
                "bytes",
                response_bytes,
            ),
            (
                "workbench.session_head_recovery_turn_count",
                "count",
                head.map(|value| value.turns.len() as f64).unwrap_or(0.0),
            ),
            (
                "workbench.session_head_recovery_message_count",
                "count",
                head.map(|value| value.messages.len() as f64).unwrap_or(0.0),
            ),
            (
                "workbench.session_head_recovery_tool_summary_count",
                "count",
                head.map(|value| value.tool_summaries.len() as f64)
                    .unwrap_or(0.0),
            ),
            (
                "workbench.session_head_recovery_event_count",
                "count",
                head.map(|value| value.events.len() as f64).unwrap_or(0.0),
            ),
        ];
        let perf_telemetry = self.state.telemetry.perf_telemetry.clone();
        tokio::spawn(async move {
            for (name, unit, value) in metrics {
                perf_telemetry
                    .record_metric(
                        PerfMetric {
                            name: name.to_string(),
                            kind: PerfMetricKind::Histogram,
                            unit: unit.to_string(),
                            value,
                            labels: labels.clone(),
                        },
                        None,
                        None,
                        None,
                    )
                    .await;
            }
        });
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        self.state.get_worktree_vcs_snapshot(worktree_id).await
    }

    pub async fn worktree_has_vcs_repo(&self, worktree: &Worktree) -> anyhow::Result<bool> {
        worktree_has_vcs_repo(&self.state, worktree).await
    }

    pub async fn load_git_status_snapshot(
        &self,
        worktree: &Worktree,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> anyhow::Result<GitStatusSnapshot> {
        load_git_status_snapshot(
            &self.state,
            worktree,
            include_untracked_files,
            include_entries,
        )
        .await
    }

    pub async fn resolve_worktree_commit(
        &self,
        worktree: &Worktree,
        revision: &str,
    ) -> anyhow::Result<String> {
        let source = HttpWorktreeVcsSource::new(&self.state, worktree);
        source.resolve_commit(revision).await
    }

    pub async fn diff_worktree_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<String> {
        diff_worktree_for_session(&self.state, worktree, base_commit_sha).await
    }

    pub async fn diff_worktree_summary_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
        diff_worktree_summary_for_session(&self.state, worktree, base_commit_sha).await
    }

    pub async fn resolve_worktree_diff_base(
        &self,
        worktree: &Worktree,
        query: WorktreeVcsDiffBaseQuery,
    ) -> WorktreeDiffBaseResolution {
        let source = HttpWorktreeVcsSource::new(&self.state, worktree);
        resolve_worktree_diff_base_from_source(&source, worktree, query).await
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

    pub async fn store_inline_image_blob(
        &self,
        bytes: &[u8],
        mime_type: &str,
        name: Option<&str>,
    ) -> Result<String, SessionImageBlobStoreError> {
        const MAX_BLOB_BYTES: usize = 25 * 1024 * 1024;

        if bytes.len() > MAX_BLOB_BYTES {
            return Err(SessionImageBlobStoreError::PayloadTooLarge);
        }
        if !mime_type.starts_with("image/") {
            return Err(SessionImageBlobStoreError::UnsupportedMediaType);
        }

        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        let sha256 = hex::encode(hasher.finalize());
        let blob_id = uuid::Uuid::new_v4().to_string();

        let dir = self.state.core.data_root.join("blobs");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|_| SessionImageBlobStoreError::Internal)?;
        let path = dir.join(&blob_id);
        let tmp = dir.join(format!("{blob_id}.tmp"));

        tokio::fs::write(&tmp, bytes)
            .await
            .map_err(|_| SessionImageBlobStoreError::Internal)?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|_| SessionImageBlobStoreError::Internal)?;

        self.state
            .global_store()
            .insert_blob(
                &blob_id,
                &sha256,
                bytes.len() as i64,
                mime_type,
                name,
                Utc::now(),
            )
            .await
            .map_err(|_| SessionImageBlobStoreError::Internal)?;

        Ok(blob_id)
    }

    pub async fn session_artifact_path_is_accessible(
        &self,
        store: &Store,
        session: &Session,
        path: &Path,
    ) -> anyhow::Result<bool> {
        let roots = self.session_artifact_allowed_roots(store, session).await?;
        let canonical = match tokio::fs::canonicalize(path).await {
            Ok(canonical) => canonical,
            Err(_) => return Ok(false),
        };
        Ok(roots.iter().any(|root| canonical.starts_with(root)))
    }

    async fn session_artifact_allowed_roots(
        &self,
        store: &Store,
        session: &Session,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let mut roots = Vec::with_capacity(2);
        if let Some(worktree) = store.get_worktree(session.worktree_id).await? {
            roots.push(canonicalize_existing_or_raw(&PathBuf::from(worktree.root_path)).await);
        }
        roots.push(
            canonicalize_existing_or_raw(
                &self
                    .state
                    .core
                    .tool_output_spool_dir
                    .join(session.id.0.to_string()),
            )
            .await,
        );
        Ok(roots)
    }
}

fn session_root_kind_for_worktree(worktree: Option<&Worktree>) -> &'static str {
    match worktree.and_then(|worktree| worktree.git_branch.as_ref()) {
        Some(_) => "worktree",
        None => "workspace_root",
    }
}

struct PersistedPostUserMessage {
    saved: Message,
    run_id: RunId,
    turn_id: TurnId,
    order_seq: i64,
}

#[derive(Debug, PartialEq, Eq)]
struct MessageAttachmentSignature {
    mime_type: String,
    name: Option<String>,
    sha256: String,
}

struct DemoSeededTurn {
    session_id: SessionId,
    run_id: RunId,
    turn_id: TurnId,
    user_message: Message,
    assistant_message: Message,
    user_created_at: chrono::DateTime<Utc>,
    assistant_created_at: chrono::DateTime<Utc>,
    user_order_seq: i64,
    assistant_order_seq: i64,
}

impl DemoSeededTurn {
    fn new(
        session_id: SessionId,
        task_id: TaskId,
        index: usize,
        base_time: chrono::DateTime<Utc>,
        turn: &DemoSeedTranscriptTurn,
    ) -> Self {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let user_created_at = base_time + ChronoDuration::seconds((index as i64) * 12);
        let assistant_created_at = user_created_at + ChronoDuration::seconds(4);
        let user_order_seq = (index as i64) * 2 + 1;
        let assistant_order_seq = user_order_seq + 1;

        let user_message = Message {
            id: MessageId::new(),
            session_id,
            task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(0),
            order_seq: Some(user_order_seq),
            role: MessageRole::User,
            content: turn.user.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(user_created_at),
            created_at: user_created_at,
        };
        let assistant_message = Message {
            id: MessageId::new(),
            session_id,
            task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(1),
            order_seq: Some(assistant_order_seq),
            role: MessageRole::Assistant,
            content: turn.assistant.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(assistant_created_at),
            created_at: assistant_created_at,
        };

        Self {
            session_id,
            run_id,
            turn_id,
            user_message,
            assistant_message,
            user_created_at,
            assistant_created_at,
            user_order_seq,
            assistant_order_seq,
        }
    }
}

async fn canonicalize_existing_or_raw(path: &Path) -> PathBuf {
    tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf())
}

fn session_store_access_anyhow(error: SessionStoreAccessError) -> anyhow::Error {
    match error {
        SessionStoreAccessError::NotFound => anyhow!("session not found"),
        SessionStoreAccessError::LookupUnavailable(error) => error,
        SessionStoreAccessError::StoreUnavailable => anyhow!("workspace store unavailable"),
    }
}

fn session_store_access_auth_error(error: SessionStoreAccessError) -> auth::SessionAuthError {
    match error {
        SessionStoreAccessError::NotFound => auth::SessionAuthError::NotFound("session"),
        SessionStoreAccessError::LookupUnavailable(error) => {
            auth::SessionAuthError::Internal(error.to_string())
        }
        SessionStoreAccessError::StoreUnavailable => {
            auth::SessionAuthError::Internal("workspace store unavailable".to_string())
        }
    }
}

fn session_store_access_mode_error(error: SessionStoreAccessError) -> SetSessionModeError {
    match error {
        SessionStoreAccessError::NotFound => SetSessionModeError::NotFound,
        SessionStoreAccessError::LookupUnavailable(_)
        | SessionStoreAccessError::StoreUnavailable => SetSessionModeError::Internal,
    }
}

fn session_store_access_model_target_error(
    error: SessionStoreAccessError,
) -> SessionModelTargetLoadError {
    match error {
        SessionStoreAccessError::NotFound => SessionModelTargetLoadError::NotFound("session"),
        SessionStoreAccessError::LookupUnavailable(error) => {
            SessionModelTargetLoadError::Internal(error)
        }
        SessionStoreAccessError::StoreUnavailable => {
            SessionModelTargetLoadError::Internal(anyhow!("workspace store unavailable"))
        }
    }
}

fn post_message_store_error(error: SessionStoreAccessError) -> PostUserMessageError {
    match error {
        SessionStoreAccessError::NotFound => {
            PostUserMessageError::NotFound("Session not found.".to_string())
        }
        SessionStoreAccessError::LookupUnavailable(_)
        | SessionStoreAccessError::StoreUnavailable => {
            PostUserMessageError::Internal("workspace store unavailable".to_string())
        }
    }
}

fn post_message_delivery_error(error: MessageDeliveryResolutionError) -> PostUserMessageError {
    match error {
        MessageDeliveryResolutionError::QueuedMessagesDisabled => {
            PostUserMessageError::BadRequest(error.message().to_string())
        }
        MessageDeliveryResolutionError::TurnAlreadyRunning => {
            PostUserMessageError::Conflict(error.message().to_string())
        }
    }
}

fn session_head_limit_bucket(limit: u32) -> &'static str {
    match limit {
        0 => "zero",
        1..=5 => "1_5",
        6..=60 => "6_60",
        61..=200 => "61_200",
        _ => "gt_200",
    }
}
