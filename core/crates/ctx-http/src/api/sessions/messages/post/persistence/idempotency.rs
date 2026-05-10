use super::super::events::ensure_existing_message_turn;
use super::*;
use crate::api::sessions::messages::attachments::attachments_match;
use ctx_session_service::message_delivery::delivery_matches;
use ctx_store::Store;

pub(in crate::api::sessions::messages::post) async fn load_matching_existing_message(
    state: &Arc<AppState>,
    store: &Store,
    session_id: SessionId,
    parts: &PostMessageParts,
) -> Result<Option<Message>, ApiErr> {
    if !parts.client_supplied_ids {
        return Ok(None);
    }
    let Some(existing) = store
        .get_message(parts.message_id)
        .await
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load message."))?
    else {
        return Ok(None);
    };
    if message_matches_request(state, &existing, session_id, parts).await? {
        ensure_existing_message_turn(store, session_id, parts.turn_id, &existing).await?;
        return Ok(Some(existing));
    }
    Err(api_error(
        StatusCode::CONFLICT,
        "A different message already exists for that client id.",
    ))
}

pub(super) async fn load_conflicting_existing_message(
    state: &Arc<AppState>,
    store: &Store,
    session_id: SessionId,
    parts: &PostMessageParts,
) -> Result<Message, ApiErr> {
    let Some(existing) = store
        .get_message(parts.message_id)
        .await
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load message."))?
    else {
        return Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Message already existed but could not be loaded.",
        ));
    };
    if message_matches_request(state, &existing, session_id, parts).await? {
        Ok(existing)
    } else {
        Err(api_error(
            StatusCode::CONFLICT,
            "A different message already exists for that client id.",
        ))
    }
}

async fn message_matches_request(
    state: &Arc<AppState>,
    existing: &Message,
    session_id: SessionId,
    parts: &PostMessageParts,
) -> Result<bool, ApiErr> {
    Ok(existing.session_id == session_id
        && existing.turn_id == Some(parts.turn_id)
        && matches!(existing.role, MessageRole::User)
        && existing.content == parts.content
        && match parts.requested_delivery.as_ref() {
            Some(requested_delivery) => delivery_matches(&existing.delivery, requested_delivery),
            None => true,
        }
        && attachments_match(state, &existing.attachments, &parts.attachments).await?)
}
