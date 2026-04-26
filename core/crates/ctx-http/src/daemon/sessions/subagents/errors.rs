use axum::http::StatusCode;
use axum::Json;

use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::logs;
use ctx_core::ids::SessionId;
use ctx_core::models::Session;

pub(super) type ApiResult<T> = Result<T, (StatusCode, Json<ApiErrorResp>)>;

pub(super) fn api_error(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

pub(super) fn internal_api_error(error: impl ToString) -> (StatusCode, Json<ApiErrorResp>) {
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        logs::redact_sensitive(&error.to_string()),
    )
}

pub(super) async fn store_for_session(
    state: &AppState,
    session_id: SessionId,
) -> ApiResult<ctx_store::Store> {
    state
        .store_for_session(session_id)
        .await
        .map_err(internal_api_error)
}

pub(super) async fn load_parent_session(
    state: &AppState,
    parent_id: SessionId,
) -> ApiResult<(ctx_store::Store, Session)> {
    let store = store_for_session(state, parent_id).await?;
    if store
        .is_archived_subagent_session(parent_id)
        .await
        .map_err(internal_api_error)?
    {
        return Err(api_error(StatusCode::NOT_FOUND, "parent session not found"));
    }
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "parent session not found"))?;
    Ok((store, parent))
}
