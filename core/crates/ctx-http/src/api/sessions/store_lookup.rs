use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::SessionId;
use ctx_observability::logs;

use super::super::errors::ApiErrorResp;
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

const STORE_OPEN_RETRY_LIMIT: usize = 3;
const STORE_OPEN_RETRY_BASE_MS: u64 = 40;

fn is_transient_store_open_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("database is locked")
        || msg.contains("sqlite_busy")
        || msg.contains("database is busy")
}

async fn store_for_existing_session_status_with_retry(
    state: &Arc<AppState>,
    session_id: SessionId,
    retry_limit: usize,
    retry_base_ms: u64,
) -> Result<ctx_store::Store, StatusCode> {
    let mut attempt = 0usize;
    loop {
        match state.lookup_session_store(session_id).await {
            StoreLookup::Found(store) => return Ok(store),
            StoreLookup::Missing | StoreLookup::Deleting => {
                return Err(StatusCode::NOT_FOUND);
            }
            StoreLookup::Unavailable(err) => {
                if is_transient_store_open_error(&err) && attempt < retry_limit {
                    attempt += 1;
                    let backoff_ms = retry_base_ms.saturating_mul(attempt as u64);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    continue;
                }
                tracing::warn!(session_id = %session_id.0, "session store lookup failed: {err:#}");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }
}

pub(in crate::api::sessions) async fn store_for_existing_session_status_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let store = store_for_existing_session_status_with_retry(
        state,
        session_id,
        STORE_OPEN_RETRY_LIMIT,
        STORE_OPEN_RETRY_BASE_MS,
    )
    .await?;
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(store)
}

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
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "workspace store unavailable".to_string(),
                }),
            )
        })?
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }
    Ok(store)
}

pub(in crate::api::sessions) async fn store_for_existing_session_api_error_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    let store = match store_for_existing_session_status_with_retry(
        state,
        session_id,
        STORE_OPEN_RETRY_LIMIT,
        STORE_OPEN_RETRY_BASE_MS,
    )
    .await
    {
        Ok(store) => store,
        Err(StatusCode::NOT_FOUND) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            ));
        }
        Err(StatusCode::INTERNAL_SERVER_ERROR) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "workspace store unavailable".to_string(),
                }),
            ));
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
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "workspace store unavailable".to_string(),
                }),
            )
        })?
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }
    Ok(store)
}
