use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_core::models::{
    Message, MessageAttachment, MessageDelivery, MessageRole, Session, SessionEvent,
    SessionEventType, SessionSummary, SessionTurn, SessionTurnStatus, SubagentInvocation, Task,
    Worktree,
};
use ctx_observability::ops_events::OpsEvent;
use ctx_observability::telemetry::TelemetryEvent;
use ctx_session_service::message_delivery::{
    build_user_message_turn, delivery_matches,
    resolve_message_delivery as resolve_message_delivery_policy, MessageDeliveryResolutionError,
};
use ctx_session_tools::model_resolution::compose_model_id;
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::{is_unique_constraint_violation, Store};
use sha2::Digest;
use tokio::sync::{mpsc, Mutex};

use super::{
    ask_user, auth, command_dispatch, subagents, title_generation,
    title_generation::{schedule_session_title_generation, TitleGenerationOutcome},
};
use crate::daemon::handle::SessionsHandle;
use crate::daemon::maintenance::post_message_update_drain_reason;
use crate::daemon::{
    require_scoped_mcp_session_context, ScopedMcpSessionAccessError, SessionStoreAccessError,
};

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

impl SessionsHandle {
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
