use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::SessionId;
use ctx_observability::logs;

use super::super::super::errors::ApiErrorResp;
use crate::daemon::{AppState, SessionStoreAccessError};

#[cfg(test)]
pub(in crate::api::sessions) async fn store_for_existing_session_api_error_allow_archived(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    state
        .existing_session_store_allow_archived(session_id)
        .await
        .map_err(read_session_store_api_error)
}

pub(in crate::api::sessions) async fn store_for_existing_session_api_error(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    state
        .existing_session_store(session_id)
        .await
        .map_err(read_session_store_api_error)
}

pub(in crate::api::sessions) async fn store_for_existing_session_api_error_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    state
        .existing_session_store_for_write(session_id)
        .await
        .map_err(write_session_store_api_error)
}

fn read_session_store_api_error(
    error: SessionStoreAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        SessionStoreAccessError::NotFound => session_not_found_error(),
        SessionStoreAccessError::LookupUnavailable(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&err.to_string()),
            }),
        ),
        SessionStoreAccessError::StoreUnavailable => workspace_store_unavailable_error(),
    }
}

fn write_session_store_api_error(
    error: SessionStoreAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        SessionStoreAccessError::NotFound => session_not_found_error(),
        SessionStoreAccessError::LookupUnavailable(_)
        | SessionStoreAccessError::StoreUnavailable => workspace_store_unavailable_error(),
    }
}

fn session_not_found_error() -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiErrorResp {
            error: "session not found".to_string(),
        }),
    )
}

fn workspace_store_unavailable_error() -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: "workspace store unavailable".to_string(),
        }),
    )
}
