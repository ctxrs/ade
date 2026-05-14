use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use ctx_observability::logs;

use crate::api::errors::ApiErrorResp;
use crate::daemon::CoreHandle;

const LOCAL_DAEMON_SHUTDOWN_TOKEN_HEADER: &str = "x-ctx-local-daemon-shutdown-token";

pub(super) fn internal_error_response(
    err: impl std::fmt::Display,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&err.to_string()),
        }),
    )
}

pub(super) fn local_shutdown_token_authorized(state: &CoreHandle, headers: &HeaderMap) -> bool {
    let Some(expected) = state.local_shutdown_token() else {
        return false;
    };
    headers
        .get(LOCAL_DAEMON_SHUTDOWN_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == expected)
}
