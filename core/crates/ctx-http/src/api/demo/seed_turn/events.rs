use serde_json::Value;

use super::records::SeededTurn;
use super::{internal_error, SeedTurnResult};
use ctx_core::models::{SessionEvent, SessionEventType};
use ctx_store::Store;

pub(super) async fn append_user_message_event(
    store: &Store,
    seeded: &SeededTurn,
) -> SeedTurnResult<SessionEvent> {
    store
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
        .map_err(|_| internal_error("failed to append user event"))
}

pub(super) async fn append_turn_started_event(
    store: &Store,
    seeded: &SeededTurn,
) -> SeedTurnResult<SessionEvent> {
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
        .map_err(|_| internal_error("failed to append turn started event"))
}

pub(super) async fn append_assistant_message_event(
    store: &Store,
    seeded: &SeededTurn,
) -> SeedTurnResult<SessionEvent> {
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
        .map_err(|_| internal_error("failed to append assistant event"))
}

pub(super) async fn append_done_event(
    store: &Store,
    seeded: &SeededTurn,
    context_window: Option<&Value>,
) -> SeedTurnResult<SessionEvent> {
    store
        .append_session_event(
            seeded.session_id,
            Some(seeded.run_id),
            Some(seeded.turn_id),
            SessionEventType::Done,
            {
                let mut payload = serde_json::json!({
                    "status": "completed",
                });
                if let Some(metrics) = context_window {
                    if let Some(obj) = payload.as_object_mut() {
                        obj.insert("context_window".to_string(), metrics.clone());
                    }
                }
                payload
            },
        )
        .await
        .map_err(|_| internal_error("failed to append done event"))
}

pub(super) async fn append_turn_finished_event(
    store: &Store,
    seeded: &SeededTurn,
) -> SeedTurnResult<SessionEvent> {
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
        .map_err(|_| internal_error("failed to append turn finished event"))
}
