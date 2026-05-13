use std::sync::Arc;

use axum::http::StatusCode;
use ctx_core::ids::SessionId;

use crate::daemon::{AppState, SessionStoreAccessError};

pub(in crate::api::sessions) async fn store_for_existing_session_status_allow_archived(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    state
        .existing_session_store_allow_archived(session_id)
        .await
        .map_err(session_store_status)
}

pub(in crate::api::sessions) async fn store_for_existing_session_status(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    state
        .existing_session_store(session_id)
        .await
        .map_err(session_store_status)
}

pub(in crate::api::sessions) async fn store_for_existing_session_status_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    state
        .existing_session_store_for_write(session_id)
        .await
        .map_err(session_store_status)
}

fn session_store_status(error: SessionStoreAccessError) -> StatusCode {
    match error {
        SessionStoreAccessError::NotFound => StatusCode::NOT_FOUND,
        SessionStoreAccessError::LookupUnavailable(_)
        | SessionStoreAccessError::StoreUnavailable => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
