use super::*;
use ctx_session_service::message_delivery::build_user_message_turn;

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

    let turn = build_user_message_turn(message, turn_id, None);
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
