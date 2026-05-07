use super::*;
use crate::scheduler::SchedulerCommand;
use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;

const LOCAL_DAEMON_SHUTDOWN_TOKEN_HEADER: &str = "x-ctx-local-daemon-shutdown-token";

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ShutdownDaemonReq {
    confirm: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct ShutdownDaemonResp {
    accepted: bool,
    activity: crate::daemon::DaemonTurnActivitySummary,
}

fn internal_error_response(err: impl std::fmt::Display) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&err.to_string()),
        }),
    )
}

fn local_shutdown_token_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = state.core.local_shutdown_token.as_deref() else {
        return false;
    };
    headers
        .get(LOCAL_DAEMON_SHUTDOWN_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == expected)
}

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

#[derive(Debug, Deserialize)]
pub(in crate::api) struct BeginUpdateDrainReq {
    confirm: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    owner: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct BeginUpdateDrainResp {
    acquired: bool,
    activity: crate::daemon::DaemonTurnActivitySummary,
}

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

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ReleaseUpdateDrainReq {
    confirm: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct ReleaseUpdateDrainResp {
    released: bool,
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
