use base64::Engine;
use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{
    Message, MessageAttachment, MessageDelivery, MessageRole, Session, SessionEventType,
    SessionTurn,
};
use ctx_session_service::message_delivery::{
    build_user_message_turn, delivery_matches,
    resolve_message_delivery as resolve_message_delivery_policy, MessageDeliveryResolutionError,
};
use ctx_store::{is_unique_constraint_violation, Store};
use sha2::Digest;

use super::command_dispatch;
use crate::daemon::handle::SessionsHandle;
use crate::daemon::maintenance::post_message_update_drain_reason;
use crate::daemon::SessionStoreAccessError;

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

    pub async fn post_message_update_drain_reason(&self) -> Option<String> {
        post_message_update_drain_reason(self.state.as_ref()).await
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
