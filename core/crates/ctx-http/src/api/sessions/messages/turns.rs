use super::*;

pub(super) fn delivery_matches(left: &MessageDelivery, right: &MessageDelivery) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

fn initial_turn_status(delivery: &MessageDelivery) -> SessionTurnStatus {
    match delivery {
        MessageDelivery::Queued => SessionTurnStatus::Queued,
        MessageDelivery::Immediate => SessionTurnStatus::Starting,
    }
}

pub(crate) async fn ensure_session_turn_for_message(
    store: &ctx_store::Store,
    session_id: SessionId,
    turn_id: TurnId,
    message: &Message,
) -> Result<(), StatusCode> {
    let existing_turn = store
        .get_session_turn_by_id(turn_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(existing) = existing_turn {
        let matches =
            existing.session_id == session_id && existing.user_message_id == Some(message.id);
        if !matches {
            return Err(StatusCode::CONFLICT);
        }
        return Ok(());
    }

    let turn = SessionTurn {
        turn_id,
        session_id,
        run_id: message.run_id,
        user_message_id: Some(message.id),
        status: initial_turn_status(&message.delivery),
        start_seq: None,
        end_seq: None,
        started_at: message.created_at,
        updated_at: message.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    if let Err(err) = store.insert_session_turn(turn).await {
        if !is_unique_constraint_violation(&err) {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let existing = store
            .get_session_turn_by_id(turn_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(existing) = existing {
            let matches =
                existing.session_id == session_id && existing.user_message_id == Some(message.id);
            if !matches {
                return Err(StatusCode::CONFLICT);
            }
        } else {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    Ok(())
}
