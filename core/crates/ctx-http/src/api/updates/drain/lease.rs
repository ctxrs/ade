use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ctx_observability::logs;

use super::types::{
    BeginUpdateDrainReq, BeginUpdateDrainResp, ReleaseUpdateDrainReq, ReleaseUpdateDrainResp,
};
use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;

pub(in crate::api) async fn begin_update_drain(
    State(state): State<Arc<AppState>>,
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
    if state
        .core
        .update_drain
        .acquire(reason, owner)
        .await
        .is_none()
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "daemon update drain already active".to_string(),
            }),
        ));
    }
    let activity = crate::daemon::daemon_turn_activity_summary(&state)
        .await
        .map_err(|err| {
            let state = state.clone();
            tokio::spawn(async move {
                let _ = state.core.update_drain.release().await;
            });
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;
    if !activity.idle {
        let _ = state.core.update_drain.release().await;
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "daemon has queued or running turns; update drain was not acquired"
                    .to_string(),
            }),
        ));
    }
    Ok(Json(BeginUpdateDrainResp {
        acquired: true,
        activity,
    }))
}

pub(in crate::api) async fn release_update_drain(
    State(state): State<Arc<AppState>>,
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
    let released = state.core.update_drain.release().await;
    Ok(Json(ReleaseUpdateDrainResp { released }))
}
