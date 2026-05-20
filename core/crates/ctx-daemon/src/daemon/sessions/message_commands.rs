use base64::Engine;
use ctx_core::ids::{MessageId, SessionId, TurnId};
use ctx_core::models::{Message, MessageAttachment, MessageDelivery, Session};
use ctx_session_service::message_admission::{
    post_user_message_record, MessageAdmissionError, MessageAttachmentSignature,
    MessageAttachmentSignatureError, MessageAttachmentSignatureResolver,
    PostUserMessageRecordInput,
};
use ctx_store::Store;
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

        let order_seq_state = self.session_order_seq_state(&store, session.id).await;
        let admission = post_user_message_record(
            &store,
            &session,
            order_seq_state,
            self,
            &PostUserMessageRecordInput {
                message_id: input.message_id,
                turn_id: input.turn_id,
                client_supplied_ids: input.client_supplied_ids,
                content: input.content,
                requested_delivery: input.requested_delivery,
                attachments: input.attachments,
                queued_messages_enabled: input.queued_messages_enabled,
                session_running: self.is_session_running(session.id).await,
            },
        )
        .await
        .map_err(post_message_admission_error)?;

        for event in admission.appended_events {
            self.publish_event(event).await;
        }
        if admission.action.should_enqueue_scheduler() {
            self.enqueue_user_message_for_scheduler(
                &store,
                session,
                admission.message.clone(),
                input.run_id_header,
            )
            .await;
        }

        Ok(admission.message)
    }

    pub async fn post_message_update_drain_reason(&self) -> Option<String> {
        post_message_update_drain_reason(self.state.as_ref()).await
    }
}

#[async_trait::async_trait]
impl MessageAttachmentSignatureResolver for SessionsHandle {
    async fn message_attachment_signature(
        &self,
        attachment: &MessageAttachment,
    ) -> Result<MessageAttachmentSignature, MessageAttachmentSignatureError> {
        match attachment {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|_| {
                        MessageAttachmentSignatureError::BadRequest(
                            "Invalid image attachment.".to_string(),
                        )
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
                        MessageAttachmentSignatureError::Internal(
                            "Failed to inspect image attachment.".to_string(),
                        )
                    })?
                else {
                    return Err(MessageAttachmentSignatureError::BadRequest(
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

fn post_message_admission_error(error: MessageAdmissionError) -> PostUserMessageError {
    match error {
        MessageAdmissionError::BadRequest(error) => PostUserMessageError::BadRequest(error),
        MessageAdmissionError::Conflict(error) => PostUserMessageError::Conflict(error),
        MessageAdmissionError::Internal(error) => PostUserMessageError::Internal(error),
    }
}
