use super::*;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_session_tools::interrupt_telemetry::{metric_labels, InterruptTelemetryContext};

pub(crate) async fn cancel_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = store_for_existing_session_status_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Cancel).await;
    Ok(StatusCode::OK)
}

pub(crate) async fn interrupt_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let request_started = std::time::Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = store_for_existing_session_status_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_root_kind = match store.get_worktree(session.worktree_id).await {
        Ok(Some(worktree)) if worktree.vcs_ref.is_some() || worktree.git_branch.is_some() => {
            "worktree"
        }
        Ok(Some(_)) => "workspace_root",
        _ => "unknown",
    };
    let provider_id = session.provider_id.clone();
    let model_id = session.model_id.clone();
    let execution_environment = session.execution_environment;
    let tx = state.ensure_scheduler(session).await;
    let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
    let _ = tx
        .send(SchedulerCommand::Interrupt(interrupt.clone()))
        .await;
    let dispatch_ms = request_started.elapsed().as_millis() as u64;
    let metric = PerfMetric {
        name: "scheduler.interrupt_http_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: dispatch_ms as f64,
        labels: metric_labels(
            &provider_id,
            &model_id,
            execution_environment.as_str(),
            session_root_kind,
            "http_dispatch",
        ),
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(metric, None, None, None)
        .await;
    tracing::info!(
        session_id = %session_id.0,
        interrupt_id = %interrupt.interrupt_id(),
        provider_id = %provider_id,
        model_id = %model_id,
        dispatch_ms,
        "session interrupt dispatched"
    );
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthenticateSessionReq {
    #[serde(default)]
    method_id: Option<String>,
}

pub(crate) async fn authenticate_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AuthenticateSessionReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = store_for_existing_session_api_error_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    crate::daemon::sessions::auth::run_session_authentication(
        &state,
        &store,
        &session,
        req.method_id,
    )
    .await
    .map_err(map_session_auth_error)?;
    Ok(StatusCode::OK)
}

fn map_session_auth_error(
    error: crate::daemon::sessions::auth::SessionAuthError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::sessions::auth::SessionAuthError::NotFound(entity) => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: format!("{entity} not found"),
            }),
        ),
        crate::daemon::sessions::auth::SessionAuthError::BadRequest(error) => {
            (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error }))
        }
        crate::daemon::sessions::auth::SessionAuthError::Forbidden(error) => {
            (StatusCode::FORBIDDEN, Json(ApiErrorResp { error }))
        }
        crate::daemon::sessions::auth::SessionAuthError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        ),
        crate::daemon::sessions::auth::SessionAuthError::AuthenticationFailed {
            redacted_message,
        } => {
            tracing::warn!("session authentication failed: {redacted_message}");
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "authentication failed".to_string(),
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_session_auth_error_maps_policy_denials_to_forbidden() {
        let (status, body) =
            map_session_auth_error(crate::daemon::sessions::auth::SessionAuthError::Forbidden(
                "host execution is disabled by daemon policy".to_string(),
            ));

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body.0.error,
            "host execution is disabled by daemon policy".to_string()
        );
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubmitAskUserQuestionReq {
    tool_call_id: String,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    answers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubmitAskUserQuestionResp {
    ok: bool,
}

pub(crate) async fn submit_ask_user_question(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubmitAskUserQuestionReq>,
) -> Result<Json<SubmitAskUserQuestionResp>, (StatusCode, Json<ApiErrorResp>)> {
    let session_uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?;
    let session_id = SessionId(session_uuid);

    // Validate the session exists (prevents accidentally fulfilling a prompt for a deleted session).
    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let exists = store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .is_some();
    if !exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }

    let tool_call_id = req.tool_call_id.trim().to_string();
    if tool_call_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "missing tool_call_id".to_string(),
            }),
        ));
    }

    let outcome = match req.outcome.as_deref() {
        Some("cancelled") => AskUserQuestionOutcome::Cancelled,
        Some("submitted") | None => AskUserQuestionOutcome::Submitted,
        Some(other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("invalid outcome: {other}"),
                }),
            ));
        }
    };
    let answers = req.answers.unwrap_or_default();
    let answers_for_event = answers.clone();

    let ok = state
        .core
        .ask_user_question
        .submit(
            &session_uuid.to_string(),
            &tool_call_id,
            AskUserQuestionAnswer { outcome, answers },
        )
        .await;

    if !ok {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "no pending AskUserQuestion for this tool_call_id".to_string(),
            }),
        ));
    }

    if let Ok(store) = state.store_for_session(session_id).await {
        if let Ok(event) = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({
                    "kind": "ask_user_question_answered",
                    "tool_call_id": tool_call_id,
                    "outcome": outcome.as_str(),
                    "answers": answers_for_event,
                }),
            )
            .await
        {
            state.publish_event(event).await;
        }
    }

    Ok(Json(SubmitAskUserQuestionResp { ok: true }))
}
