use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ctx_observability::logs;

use super::types::{
    BeginUpdateDrainReq, BeginUpdateDrainResp, ReleaseUpdateDrainReq, ReleaseUpdateDrainResp,
};
use crate::api::errors::ApiErrorResp;
use ctx_daemon::daemon::{maintenance as daemon_maintenance, ExecutionHandle};

pub(in crate::api) async fn begin_update_drain(
    State(execution): State<ExecutionHandle>,
    Json(req): Json<BeginUpdateDrainReq>,
) -> Result<Json<BeginUpdateDrainResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !req.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "confirm required".to_string(),
            }),
        ));
    }
    let reason = req
        .reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "daemon_update".to_string());
    let owner = req
        .owner
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let activity = execution
        .begin_update_drain(reason, owner)
        .await
        .map_err(begin_update_drain_error)?;
    Ok(Json(BeginUpdateDrainResp {
        acquired: true,
        activity,
    }))
}

pub(in crate::api) async fn release_update_drain(
    State(execution): State<ExecutionHandle>,
    Json(req): Json<ReleaseUpdateDrainReq>,
) -> Result<Json<ReleaseUpdateDrainResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !req.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "confirm required".to_string(),
            }),
        ));
    }
    let released = execution.release_update_drain().await;
    Ok(Json(ReleaseUpdateDrainResp { released }))
}

fn begin_update_drain_error(
    error: daemon_maintenance::BeginUpdateDrainError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        daemon_maintenance::BeginUpdateDrainError::AlreadyActive => (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "daemon update drain already active".to_string(),
            }),
        ),
        daemon_maintenance::BeginUpdateDrainError::ActivityUnavailable(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        ),
        daemon_maintenance::BeginUpdateDrainError::Busy => (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "daemon has queued or running turns; update drain was not acquired"
                    .to_string(),
            }),
        ),
    }
}
