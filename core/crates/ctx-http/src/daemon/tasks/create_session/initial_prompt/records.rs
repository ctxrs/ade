use serde_json::{json, Value};

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionTurn, SessionTurnStatus,
};

use super::TaskSessionCreateError;

#[derive(Clone, Copy)]
pub(super) struct InitialPromptIds {
    pub(super) message_id: MessageId,
    pub(super) turn_id: TurnId,
}

pub(super) fn parse_initial_prompt_ids(
    message_id: Option<&str>,
    turn_id: Option<&str>,
) -> Result<InitialPromptIds, TaskSessionCreateError> {
    match (message_id, turn_id) {
        (Some(message_id), Some(turn_id)) => Ok(InitialPromptIds {
            message_id: MessageId(
                uuid::Uuid::parse_str(message_id)
                    .map_err(|_| TaskSessionCreateError::BadRequest)?,
            ),
            turn_id: TurnId(
                uuid::Uuid::parse_str(turn_id).map_err(|_| TaskSessionCreateError::BadRequest)?,
            ),
        }),
        _ => Err(TaskSessionCreateError::BadRequest),
    }
}

pub(super) fn existing_initial_prompt_message_matches(
    existing: &Message,
    session: &Session,
    turn_id: TurnId,
    prompt: &str,
) -> bool {
    existing.session_id == session.id
        && existing.turn_id == Some(turn_id)
        && matches!(existing.role, MessageRole::User)
        && existing.content == prompt
        && existing.attachments.is_empty()
        && matches!(existing.delivery, MessageDelivery::Immediate)
}

pub(super) fn new_initial_prompt_message(
    session: &Session,
    ids: InitialPromptIds,
    run_id: RunId,
    prompt: String,
    order_seq: i64,
    delivery: MessageDelivery,
) -> Message {
    Message {
        id: ids.message_id,
        session_id: session.id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(ids.turn_id),
        turn_sequence: Some(0),
        order_seq: Some(order_seq),
        role: MessageRole::User,
        content: prompt,
        attachments: Vec::new(),
        delivery,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    }
}

pub(super) fn initial_prompt_user_event_payload(saved: &Message, order_seq: i64) -> Value {
    json!({
        "message_id": saved.id.0,
        "content": saved.content.clone(),
        "delivery": saved.delivery.clone(),
        "attachments": saved.attachments,
        "order_seq": order_seq,
    })
}

pub(super) fn initial_prompt_turn(
    session: &Session,
    ids: InitialPromptIds,
    run_id: RunId,
    saved: &Message,
    start_seq: i64,
) -> SessionTurn {
    SessionTurn {
        turn_id: ids.turn_id,
        session_id: session.id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: SessionTurnStatus::Starting,
        start_seq: Some(start_seq),
        end_seq: None,
        started_at: saved.created_at,
        updated_at: saved.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        failure: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    }
}
