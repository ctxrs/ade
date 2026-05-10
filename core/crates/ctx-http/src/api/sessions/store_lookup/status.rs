use std::sync::Arc;

use axum::http::StatusCode;
use ctx_core::ids::SessionId;

use super::retry::store_for_existing_session_status_with_retry;
use crate::daemon::{AppState, StoreLookup};

pub(in crate::api::sessions) async fn store_for_existing_session_status_allow_archived(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let store = match state.lookup_session_store(session_id).await {
        StoreLookup::Found(store) => store,
        StoreLookup::Missing | StoreLookup::Deleting => {
            return Err(StatusCode::NOT_FOUND);
        }
        StoreLookup::Unavailable(_) => {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    Ok(store)
}

pub(in crate::api::sessions) async fn store_for_existing_session_status(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let store = store_for_existing_session_status_allow_archived(state, session_id).await?;
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(store)
}

pub(in crate::api::sessions) async fn store_for_existing_session_status_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let store = store_for_existing_session_status_with_retry(state, session_id).await?;
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(store)
}
