use super::*;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::scheduler::{metric_labels, InterruptTelemetryContext};

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
        interrupt_id = %interrupt.interrupt_id,
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

    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
    let install_target =
        crate::execution_effective::effective_install_target(state.as_ref(), worktree.workspace_id)
            .await
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to load workspace execution settings: {err:#}"),
                    }),
                )
            })?;
    let adapter = crate::daemon::ensure_provider_adapter_for_target(
        state.as_ref(),
        &session.provider_id,
        install_target,
    )
    .await;

    let workdir = PathBuf::from(worktree.root_path.clone());

    let mut provider_env = std::collections::HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    if let Ok(v) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CTX_MCP_DISABLED") {
        provider_env.insert("CTX_MCP_DISABLED".to_string(), v);
    }
    if session.provider_id == "codex" {
        if let Ok(extra) =
            provider_accounts::codex_env_for_active_account(&state.core.data_root).await
        {
            for (key, value) in extra {
                provider_env.insert(key, value);
            }
        }
    }

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let state_for_events = state.clone();
    let store_for_events = store.clone();
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            let mut payload = ev.payload_json.clone();
            if matches!(ev.event_type, SessionEventType::Init) {
                if payload.get("crp_session_id").is_some() {
                    state_for_events
                        .emit_compat_payload_reject_counter(
                            "sessions.auth_event_init",
                            "crp_session_id",
                            None,
                        )
                        .await;
                }
                if let Some(ps) = payload
                    .get("provider_session_id")
                    .and_then(serde_json::Value::as_str)
                {
                    let _ = store_for_events
                        .update_session_provider_session_ref(session_id, Some(ps.to_string()))
                        .await;
                }
            }
            if payload.is_object() {
                let should_attach = matches!(
                    ev.event_type,
                    SessionEventType::UserMessage
                        | SessionEventType::AssistantChunk
                        | SessionEventType::AssistantComplete
                        | SessionEventType::AssistantMessageInserted
                        | SessionEventType::ThoughtChunk
                        | SessionEventType::ToolCall
                        | SessionEventType::ToolCallUpdate
                        | SessionEventType::ToolResult
                ) || (matches!(ev.event_type, SessionEventType::Notice)
                    && payload
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|kind| {
                            kind == "reasoning_summary" || kind == "ask_user_question"
                        }));
                if should_attach {
                    let order_seq_state = state_for_events
                        .sessions
                        .get_order_seq_state(&store_for_events, session_id)
                        .await;
                    let mut order_seq_state = order_seq_state.lock().await;
                    attach_order_seq(&mut order_seq_state, &ev.event_type, &mut payload, None, 0);
                }
            }
            let appended = store_for_events
                .append_session_event(session_id, None, None, ev.event_type.clone(), payload)
                .await;
            if let Ok(event) = appended {
                state_for_events.publish_event(event).await;
            }
        }
    });

    let started = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "auth_started",
                "provider": session.provider_id,
                "method_id": req.method_id,
            }),
        )
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to append auth event".to_string(),
                }),
            )
        })?;
    state.publish_event(started).await;

    let session_key = session.id.0.to_string();
    let result = adapter
        .authenticate_session(
            session_key,
            workdir,
            provider_env,
            req.method_id.clone(),
            ev_tx,
        )
        .await;

    match result {
        Ok(()) => {
            let done = store
                .append_session_event(
                    session_id,
                    None,
                    None,
                    SessionEventType::Notice,
                    serde_json::json!({
                        "kind": "auth_finished",
                        "provider": session.provider_id,
                    }),
                )
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to append auth event".to_string(),
                        }),
                    )
                })?;
            state.publish_event(done).await;
            Ok(StatusCode::OK)
        }
        Err(e) => {
            let msg = logs::redact_sensitive(&e.to_string());
            let failed = store
                .append_session_event(
                    session_id,
                    None,
                    None,
                    SessionEventType::Notice,
                    serde_json::json!({
                        "kind": "auth_failed",
                        "provider": session.provider_id,
                        "message": msg,
                    }),
                )
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to append auth event".to_string(),
                        }),
                    )
                })?;
            state.publish_event(failed).await;
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "authentication failed".to_string(),
                }),
            ))
        }
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
