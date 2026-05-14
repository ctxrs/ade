use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use super::helpers::{internal_error_response, local_shutdown_token_authorized};
use super::types::{ShutdownDaemonReq, ShutdownDaemonResp};
use crate::api::errors::ApiErrorResp;
use ctx_daemon::daemon::{maintenance as daemon_maintenance, CoreHandle, ExecutionHandle};

pub(in crate::api) async fn shutdown_daemon(
    State(core): State<CoreHandle>,
    State(execution): State<ExecutionHandle>,
    headers: HeaderMap,
    Json(req): Json<ShutdownDaemonReq>,
) -> Result<Json<ShutdownDaemonResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !req.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "confirm required".to_string(),
            }),
        ));
    }
    if !local_shutdown_token_authorized(&core, &headers) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorResp {
                error: "local desktop shutdown token required".to_string(),
            }),
        ));
    }

    let reason = req
        .reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "desktop_quit".to_string());
    let activity = execution
        .request_daemon_shutdown(reason)
        .await
        .map_err(daemon_shutdown_error)?;
    Ok(Json(ShutdownDaemonResp {
        accepted: true,
        activity,
    }))
}

fn daemon_shutdown_error(
    error: daemon_maintenance::DaemonShutdownError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        daemon_maintenance::DaemonShutdownError::ActivityUnavailable(error)
        | daemon_maintenance::DaemonShutdownError::Reconcile(error) => {
            internal_error_response(error)
        }
    }
}
