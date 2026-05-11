use std::sync::Arc;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::{Message, Session, SessionEventType};

use crate::daemon::sessions::subagents::errors::{internal_api_error, ApiResult};
use crate::daemon::AppState;

pub(super) async fn append_and_publish_queued_prompt_events(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &Session,
    run_id: RunId,
    turn_id: TurnId,
    saved: &Message,
) -> ApiResult<i64> {
    let queued = store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::InputQueued,
            serde_json::json!({"message_id": saved.id.0}),
        )
        .await
        .map_err(internal_api_error)?;
    state.publish_event(queued).await;

    let queue_position = store
        .list_queued_messages_for_session(session.id)
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
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::MessageQueueAdded,
            serde_json::json!({
                "message_id": saved.id.0,
                "queue_position": queue_position,
            }),
        )
        .await
        .map_err(internal_api_error)?;
    state.publish_event(queue_added).await;

    let turn_queued = store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::TurnQueued,
            serde_json::json!({
                "message_id": saved.id.0,
                "queue_position": queue_position,
            }),
        )
        .await
        .map_err(internal_api_error)?;
    let last_event_seq = turn_queued.seq;
    state.publish_event(turn_queued).await;
    Ok(last_event_seq)
}
