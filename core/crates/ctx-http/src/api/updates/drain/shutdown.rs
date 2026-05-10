use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;

use super::helpers::{internal_error_response, local_shutdown_token_authorized};
use super::types::{ShutdownDaemonReq, ShutdownDaemonResp};
use crate::api::errors::ApiErrorResp;
use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::AppState;

pub(in crate::api) async fn shutdown_daemon(
    State(state): State<Arc<AppState>>,
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
    if !local_shutdown_token_authorized(&state, &headers) {
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
    let acquired_drain = state
        .core
        .update_drain
        .acquire(&reason, "daemon_shutdown")
        .await
        .is_some();

    for session_id in state.sessions.list_running_sessions().await {
        if let Some(tx) = state.sessions.scheduler_sender(session_id).await {
            let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
            let _ = tx.send(SchedulerCommand::Interrupt(interrupt)).await;
        }
    }

    for _ in 0..10 {
        let activity = match crate::daemon::daemon_turn_activity_summary(&state).await {
            Ok(activity) => activity,
            Err(err) => {
                if acquired_drain {
                    let _ = state.core.update_drain.release().await;
                }
                return Err(internal_error_response(err));
            }
        };
        if activity.running_turn_count == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    if let Err(err) = crate::daemon::reconcile_running_turns_with_reason(&state, &reason).await {
        if acquired_drain {
            let _ = state.core.update_drain.release().await;
        }
        return Err(internal_error_response(err));
    }
    let activity = match crate::daemon::daemon_turn_activity_summary(&state).await {
        Ok(activity) => activity,
        Err(err) => {
            if acquired_drain {
                let _ = state.core.update_drain.release().await;
            }
            return Err(internal_error_response(err));
        }
    };

    crate::daemon::spawn_deferred_daemon_shutdown(
        state,
        reason,
        std::time::Duration::from_millis(100),
    );
    Ok(Json(ShutdownDaemonResp {
        accepted: true,
        activity,
    }))
}
