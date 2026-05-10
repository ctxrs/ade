use super::*;

pub(super) async fn publish_queue_events(
    state: &Arc<AppState>,
    store: &Store,
    session_id: SessionId,
    persisted: &PersistedPostMessage,
) -> Result<(), ApiErr> {
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
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to append queued-input event.",
            )
        })?;
    state.publish_event(queued).await;

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
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to append queue event.",
            )
        })?;
    state.publish_event(queue_added).await;

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
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to append queued turn event.",
            )
        })?;
    state.publish_event(turn_queued).await;

    Ok(())
}
