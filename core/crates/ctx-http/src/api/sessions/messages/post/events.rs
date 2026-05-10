use super::super::turns::ensure_session_turn_for_message;
use super::super::*;
use super::persistence::PersistedPostMessage;
use ctx_session_service::message_delivery::build_user_message_turn;
use ctx_store::Store;

#[path = "events/queue.rs"]
mod queue;

use self::queue::publish_queue_events;

pub(super) async fn publish_user_message_events(
    state: &Arc<AppState>,
    store: &Store,
    session_id: SessionId,
    persisted: &PersistedPostMessage,
) -> Result<(), ApiErr> {
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
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to append session event.",
            )
        })?;
    let start_seq = event.seq;

    let turn = build_user_message_turn(saved, persisted.turn_id, Some(start_seq));
    let existing_turn = store
        .get_session_turn_by_id(persisted.turn_id)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to inspect session turn.",
            )
        })?;
    if let Some(existing) = existing_turn {
        let matches =
            existing.session_id == session_id && existing.user_message_id == Some(saved.id);
        if !matches {
            return Err(api_error(
                StatusCode::CONFLICT,
                "Turn id already belongs to another message.",
            ));
        }
    } else if let Err(err) = store.insert_session_turn(turn).await {
        if !is_unique_constraint_violation(&err) {
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create session turn.",
            ));
        }
        let existing = store
            .get_session_turn_by_id(persisted.turn_id)
            .await
            .map_err(|_| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to inspect session turn.",
                )
            })?;
        if let Some(existing) = existing {
            let matches =
                existing.session_id == session_id && existing.user_message_id == Some(saved.id);
            if !matches {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    "Turn id already belongs to another message.",
                ));
            }
        } else {
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Session turn insert succeeded but could not be reloaded.",
            ));
        }
    }
    state.publish_event(event).await;

    if matches!(saved.delivery, MessageDelivery::Queued) {
        publish_queue_events(state, store, session_id, persisted).await?;
    }

    Ok(())
}

pub(super) async fn ensure_existing_message_turn(
    store: &Store,
    session_id: SessionId,
    turn_id: TurnId,
    existing: &Message,
) -> Result<(), ApiErr> {
    ensure_session_turn_for_message(store, session_id, turn_id, existing)
        .await
        .map_err(|status| match status {
            StatusCode::CONFLICT => api_error(
                StatusCode::CONFLICT,
                "Turn id already belongs to another message.",
            ),
            _ => api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to ensure message turn.",
            ),
        })
}
