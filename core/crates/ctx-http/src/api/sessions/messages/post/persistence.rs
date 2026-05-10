use super::super::attachments::normalize_message_attachments;
use super::super::*;
use super::delivery::{queued_messages_enabled, resolve_message_delivery};
use super::request::PostMessageReq;
use ctx_session_service::message_delivery::{
    resolve_message_client_ids, MessageClientIdResolutionError,
};
use ctx_store::Store;

#[path = "persistence/idempotency.rs"]
mod idempotency;

use self::idempotency::load_conflicting_existing_message;
pub(super) use self::idempotency::load_matching_existing_message;

pub(super) struct PostMessageParts {
    pub(super) message_id: MessageId,
    pub(super) turn_id: TurnId,
    pub(super) client_supplied_ids: bool,
    pub(super) content: String,
    pub(super) requested_delivery: Option<MessageDelivery>,
    pub(super) attachments: Vec<MessageAttachment>,
}

impl PostMessageParts {
    pub(super) async fn from_request(
        state: &Arc<AppState>,
        req: PostMessageReq,
    ) -> Result<Self, ApiErr> {
        let request_message_id = req
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                uuid::Uuid::parse_str(value)
                    .map(MessageId)
                    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid message id."))
            })
            .transpose()?;
        let request_turn_id = req
            .turn_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                uuid::Uuid::parse_str(value)
                    .map(TurnId)
                    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid turn id."))
            })
            .transpose()?;
        let client_ids =
            resolve_message_client_ids(request_message_id, request_turn_id).map_err(|error| {
                match error {
                    MessageClientIdResolutionError::PartialClientIds => {
                        api_error(StatusCode::BAD_REQUEST, error.message())
                    }
                }
            })?;
        Ok(Self {
            message_id: client_ids.message_id,
            turn_id: client_ids.turn_id,
            client_supplied_ids: client_ids.client_supplied,
            content: req.content,
            requested_delivery: req.delivery,
            attachments: normalize_message_attachments(state, req.attachments).await?,
        })
    }
}

pub(super) struct PersistedPostMessage {
    pub(super) saved: Message,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) order_seq: i64,
}

pub(super) async fn persist_user_message(
    state: &Arc<AppState>,
    store: &Store,
    session: &Session,
    parts: PostMessageParts,
) -> Result<PersistedPostMessage, ApiErr> {
    let delivery = resolve_message_delivery(
        parts.requested_delivery.clone(),
        state.sessions.is_running(session.id).await,
        queued_messages_enabled(),
    )?;

    let run_id = RunId::new();
    let order_seq_state = state.sessions.get_order_seq_state(store, session.id).await;
    let order_seq = {
        let mut order_seq_state = order_seq_state.lock().await;
        order_seq_state.get_or_assign(format!("message:{}", parts.message_id.0), None)
    };
    let msg = Message {
        id: parts.message_id,
        session_id: session.id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(parts.turn_id),
        turn_sequence: Some(0),
        order_seq: Some(order_seq),
        role: MessageRole::User,
        content: parts.content.clone(),
        attachments: parts.attachments.clone(),
        delivery,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = match store.insert_message(msg).await {
        Ok(saved) => saved,
        Err(err) if parts.client_supplied_ids && is_unique_constraint_violation(&err) => {
            load_conflicting_existing_message(state, store, session.id, &parts).await?
        }
        Err(_) => {
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to save message.",
            ))
        }
    };
    Ok(PersistedPostMessage {
        saved,
        run_id,
        turn_id: parts.turn_id,
        order_seq,
    })
}
