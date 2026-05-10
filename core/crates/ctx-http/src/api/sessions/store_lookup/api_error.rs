use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::SessionId;
use ctx_observability::logs;

use super::super::super::errors::ApiErrorResp;
use super::retry::store_for_existing_session_status_with_retry;
use crate::daemon::{AppState, StoreLookup};

pub(in crate::api::sessions) async fn store_for_existing_session_api_error_allow_archived(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    let store = match state.lookup_session_store(session_id).await {
        StoreLookup::Found(store) => store,
        StoreLookup::Missing | StoreLookup::Deleting => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            ));
        }
        StoreLookup::Unavailable(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            ));
        }
    };
    Ok(store)
}

pub(in crate::api::sessions) async fn store_for_existing_session_api_error(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    let store = store_for_existing_session_api_error_allow_archived(state, session_id).await?;
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| workspace_store_unavailable_error())?
    {
        return Err(session_not_found_error());
    }
    Ok(store)
}

pub(in crate::api::sessions) async fn store_for_existing_session_api_error_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    let store = match store_for_existing_session_status_with_retry(state, session_id).await {
        Ok(store) => store,
        Err(StatusCode::NOT_FOUND) => {
            return Err(session_not_found_error());
        }
        Err(StatusCode::INTERNAL_SERVER_ERROR) => {
            return Err(workspace_store_unavailable_error());
        }
        Err(status) => {
            return Err((
                status,
                Json(ApiErrorResp {
                    error: status.to_string(),
                }),
            ));
        }
    };
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| workspace_store_unavailable_error())?
    {
        return Err(session_not_found_error());
    }
    Ok(store)
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
