use super::helpers::{
    read_codex_context_window_metrics, should_track_thought_chunk, strip_emitted_prefix,
};
use super::*;
use crate::scheduler::terminal::{finalize_failed_turn, FailedTurnTerminalization};
use crate::scheduler::{latency_bucket, metric_labels, TurnStartProgress};
use crate::storage_guard;

pub(super) struct TurnEventLoop {
    pub(super) state: Arc<AppState>,
    pub(super) store: ctx_store::Store,
    pub(super) session_id: ctx_core::ids::SessionId,
    pub(super) task_id: ctx_core::ids::TaskId,
    pub(super) workspace_id: ctx_core::ids::WorkspaceId,
    pub(super) worktree_id: ctx_core::ids::WorktreeId,
    pub(super) provider_id: String,
    pub(super) model_id: String,
    pub(super) session_root_kind: String,
    pub(super) execution_environment_label: String,
    pub(super) perf_run_id: Option<String>,
    pub(super) workdir_root: PathBuf,
    pub(super) workdir_canonical: Option<PathBuf>,
    pub(super) workdir_str: String,
    pub(super) run_started_at: Instant,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) message_id: MessageId,
    pub(super) provider_session_ref: Option<String>,
    pub(super) codex_home: Option<PathBuf>,
    pub(super) context_window_metrics: Option<Value>,
    pub(super) ev_rx: mpsc::Receiver<NormalizedEvent>,
    pub(super) events_done_tx: oneshot::Sender<()>,
    pub(super) start_progress_tx: tokio::sync::watch::Sender<TurnStartProgress>,
    pub(super) order_seq_state: Arc<Mutex<OrderSeqState>>,
}

pub(super) fn spawn_turn_event_loop(ctx: TurnEventLoop) {
    tokio::spawn(async move {
        run_turn_event_loop(ctx).await;
    });
}

#[allow(clippy::too_many_arguments)]
async fn record_failed_turn_telemetry(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    worktree_id: ctx_core::ids::WorktreeId,
    run_id: RunId,
    turn_id: TurnId,
    provider_id: &str,
    model_id: &str,
    execution_environment_label: &str,
    session_root_kind: &str,
    workdir_str: &str,
    run_started_at: Instant,
    perf_run_id: Option<&String>,
    telemetry_emitted: &mut bool,
    error_message: String,
    details: Option<Value>,
    kind: Option<Value>,
) {
    if !*telemetry_emitted {
        *telemetry_emitted = true;
        let duration_ms = run_started_at.elapsed().as_millis() as u64;
        let mut run_labels = HashMap::new();
        run_labels.insert("provider_id".to_string(), provider_id.to_string());
        run_labels.insert("model_id".to_string(), model_id.to_string());
        run_labels.insert(
            "execution_environment".to_string(),
            execution_environment_label.to_string(),
        );
        run_labels.insert(
            "session_root_kind".to_string(),
            session_root_kind.to_string(),
        );
        run_labels.insert("event".to_string(), "run_failed".to_string());
        let run_metric = PerfMetric {
            name: "scheduler.run_total_ms".to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: duration_ms as f64,
            labels: run_labels,
        };
        state
            .telemetry
            .perf_telemetry
            .record_metric(run_metric, perf_run_id.cloned(), None, None)
            .await;
        state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::provider_call(
                provider_id.to_string(),
                model_id.to_string(),
                Some(execution_environment_label.to_string()),
                Some(session_root_kind.to_string()),
                false,
                duration_ms,
            ))
            .await;
        state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::session_completed(
                provider_id.to_string(),
                model_id.to_string(),
                Some(execution_environment_label.to_string()),
                Some(session_root_kind.to_string()),
                "failed".to_string(),
                duration_ms,
            ))
            .await;
    }

    let mut fail_event = OpsEvent::new("error", "provider_run_failed");
    fail_event.session_id = Some(session_id.0.to_string());
    fail_event.worktree_id = Some(worktree_id.0.to_string());
    fail_event.run_id = Some(run_id.0.to_string());
    fail_event.turn_id = Some(turn_id.0.to_string());
    fail_event.provider_id = Some(provider_id.to_string());
    fail_event.cwd = Some(workdir_str.to_string());
    fail_event.worktree_root = Some(workdir_str.to_string());
    fail_event.meta = Some(json!({
        "model_id": model_id,
        "execution_environment": execution_environment_label,
        "session_root_kind": session_root_kind,
        "error": error_message.clone(),
        "details": details.clone(),
        "kind": kind.clone(),
    }));
    state.telemetry.ops_events.emit(fail_event);
}

async fn fail_turn(ctx: TurnFailureContext<'_>, failure: TurnFailurePayload) {
    record_failed_turn_telemetry(
        ctx.state,
        ctx.session_id,
        ctx.worktree_id,
        ctx.run_id,
        ctx.turn_id,
        ctx.provider_id,
        ctx.model_id,
        ctx.execution_environment_label,
        ctx.session_root_kind,
        ctx.workdir_str,
        ctx.run_started_at,
        ctx.perf_run_id,
        ctx.telemetry_emitted,
        failure.error_message.clone(),
        failure.details.clone(),
        failure.kind.clone(),
    )
    .await;
    *ctx.terminal_status = Some(SessionTurnStatus::Failed);
    let _ = finalize_failed_turn(
        ctx.state,
        ctx.session_id,
        Some(ctx.run_id),
        ctx.turn_id,
        ctx.message_id,
        FailedTurnTerminalization {
            message: &failure.error_message,
            reason: None,
            details: failure.details,
            kind: failure.kind,
            emit_error_event: ctx.emit_error_event,
        },
    )
    .await;
}

struct TurnFailureContext<'a> {
    state: &'a Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    worktree_id: ctx_core::ids::WorktreeId,
    run_id: RunId,
    turn_id: TurnId,
    message_id: MessageId,
    provider_id: &'a str,
    model_id: &'a str,
    execution_environment_label: &'a str,
    session_root_kind: &'a str,
    workdir_str: &'a str,
    run_started_at: Instant,
    perf_run_id: Option<&'a String>,
    telemetry_emitted: &'a mut bool,
    terminal_status: &'a mut Option<SessionTurnStatus>,
    emit_error_event: bool,
}

struct TurnFailurePayload {
    error_message: String,
    details: Option<Value>,
    kind: Option<Value>,
}

fn is_truthful_start_activity(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::TurnStarted
            | SessionEventType::AssistantChunk
            | SessionEventType::ThoughtChunk
            | SessionEventType::AssistantComplete
            | SessionEventType::ContextWindowUpdate
            | SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult
            | SessionEventType::Done
            | SessionEventType::TurnInterrupted
            | SessionEventType::Error
    )
}

async fn run_turn_event_loop(ctx: TurnEventLoop) {
    let TurnEventLoop {
        state,
        store,
        session_id,
        task_id,
        workspace_id,
        worktree_id,
        provider_id,
        model_id,
        session_root_kind,
        execution_environment_label,
        perf_run_id,
        workdir_root,
        workdir_canonical,
        workdir_str,
        run_started_at,
        run_id,
        turn_id,
        message_id,
        mut provider_session_ref,
        codex_home,
        context_window_metrics,
        mut ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state,
    } = ctx;

    let mut assistant_partial = String::new();
    let mut assistant_partial_message_id: Option<String> = None;
    let mut assistant_sequence: i64 = 0;
    let mut assistant_emitted = String::new();
    let mut thought_partial = String::new();
    let mut tool_cache: HashMap<String, SessionTurnTool> = HashMap::new();
    let mut terminal_status: Option<SessionTurnStatus> = None;
    let mut start_progress = TurnStartProgress::Pending;
    let mut first_event_at: Option<Instant> = None;
    let mut telemetry_emitted = false;

    while let Some(ev) = ev_rx.recv().await {
        let mut event_type = ev.event_type.clone();
        let raw_payload = ev.payload_json.clone();
        let mut payload = raw_payload.clone();
        if first_event_at.is_none() {
            first_event_at = Some(Instant::now());
            let first_ms = run_started_at.elapsed().as_millis() as u64;
            let mut first_labels = HashMap::new();
            first_labels.insert("provider_id".to_string(), provider_id.clone());
            first_labels.insert("model_id".to_string(), model_id.clone());
            first_labels.insert(
                "execution_environment".to_string(),
                execution_environment_label.clone(),
            );
            first_labels.insert("session_root_kind".to_string(), session_root_kind.clone());
            first_labels.insert("event".to_string(), "first_event".to_string());
            let first_metric = PerfMetric {
                name: "provider.first_event_ms".to_string(),
                kind: PerfMetricKind::Histogram,
                unit: "ms".to_string(),
                value: first_ms as f64,
                labels: first_labels,
            };
            state
                .telemetry
                .perf_telemetry
                .record_metric(first_metric, perf_run_id.clone(), None, None)
                .await;
        }
        if matches!(ev.event_type, SessionEventType::Init) {
            if payload.get("crp_session_id").is_some() {
                state
                    .emit_compat_payload_reject_counter(
                        "scheduler.init_event",
                        "crp_session_id",
                        None,
                    )
                    .await;
            }
            let provider_session_id = payload.get("provider_session_id").and_then(Value::as_str);
            if let Some(ps) = provider_session_id {
                match store
                    .claim_session_provider_session_ref(
                        session_id,
                        ps.to_string(),
                        "scheduler.init_event",
                    )
                    .await
                {
                    Ok(()) => {
                        provider_session_ref = Some(ps.to_string());
                    }
                    Err(err) => {
                        event_type = SessionEventType::Error;
                        payload = json!({
                            "message": err.to_string(),
                            "reason": "provider_session_ref_claim_failed",
                            "kind": "provider_session_ref_claim_failed",
                            "details": {
                                "provider_session_id": ps,
                                "provider_id": provider_id.clone(),
                            },
                        });
                    }
                }
            }
        }
        if matches!(ev.event_type, SessionEventType::Done) {
            if let Some(obj) = payload.as_object_mut() {
                if obj.get("context_window").is_none() {
                    let metrics = if provider_id == "codex-crp" {
                        codex_home
                            .as_deref()
                            .and_then(|home| {
                                provider_session_ref.as_deref().and_then(|session_ref| {
                                    read_codex_context_window_metrics(home, session_ref)
                                })
                            })
                            .or_else(|| context_window_metrics.clone())
                    } else {
                        context_window_metrics.clone()
                    };
                    if let Some(metrics) = metrics {
                        obj.entry("context_window").or_insert(metrics);
                    }
                }
                obj.entry("status").or_insert(json!("completed"));
            }
        }
        let normalized_tool_event = if matches!(
            event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        ) {
            Some(normalize_tool_event(&event_type, &raw_payload))
        } else {
            None
        };
        if matches!(event_type, SessionEventType::ToolCall) {
            if let Some(tool_event) = normalized_tool_event.as_ref() {
                let tool_meta = build_tool_ops_meta_from_normalized(tool_event);
                let mut meta = serde_json::Map::new();
                if let Some(tool_call_id) = tool_meta.tool_call_id.clone() {
                    meta.insert("tool_call_id".to_string(), json!(tool_call_id));
                }
                if let Some(title) = tool_meta.title.clone() {
                    meta.insert("title".to_string(), json!(title));
                }
                if let Some(status) = tool_meta.status.clone() {
                    meta.insert("status".to_string(), json!(status));
                }
                if let Some(input_preview) = tool_meta.input_preview.clone() {
                    meta.insert("input".to_string(), input_preview);
                }
                let mut event = OpsEvent::new("info", "tool_exec");
                event.session_id = Some(session_id.0.to_string());
                event.worktree_id = Some(worktree_id.0.to_string());
                event.run_id = Some(run_id.0.to_string());
                event.turn_id = Some(turn_id.0.to_string());
                event.provider_id = Some(provider_id.clone());
                event.tool_kind = tool_meta.tool_kind.clone();
                event.cwd = tool_meta.cwd.clone();
                event.worktree_root = Some(workdir_str.clone());
                event.meta = if meta.is_empty() {
                    None
                } else {
                    Some(Value::Object(meta))
                };
                state.telemetry.ops_events.emit(event);

                if let Some(cwd) = tool_meta.cwd.as_deref() {
                    if cwd_outside_worktree(cwd, &workdir_root, workdir_canonical.as_ref()) {
                        let mut warn_event = OpsEvent::new("warn", "tool_exec_anomaly");
                        warn_event.session_id = Some(session_id.0.to_string());
                        warn_event.worktree_id = Some(worktree_id.0.to_string());
                        warn_event.run_id = Some(run_id.0.to_string());
                        warn_event.turn_id = Some(turn_id.0.to_string());
                        warn_event.provider_id = Some(provider_id.clone());
                        warn_event.tool_kind = tool_meta.tool_kind.clone();
                        warn_event.cwd = Some(cwd.to_string());
                        warn_event.worktree_root = Some(workdir_str.clone());
                        warn_event.meta = Some(json!({
                            "reason": "cwd_outside_worktree",
                            "tool_call_id": tool_meta.tool_call_id,
                        }));
                        state.telemetry.ops_events.emit(warn_event);
                    }
                }
            }
        }
        if let Some(tool_event) = normalized_tool_event.as_ref() {
            let output_artifact = if matches!(event_type, SessionEventType::ToolResult) {
                maybe_spool_tool_output(
                    state.as_ref(),
                    &store,
                    tool_event,
                    tool_runtime::ToolOutputArtifactScope {
                        session_id,
                        task_id,
                        workspace_id,
                        worktree_id,
                        turn_id,
                    },
                )
                .await
            } else {
                None
            };
            payload = sanitize_normalized_tool_event_payload(tool_event, output_artifact.as_ref());
        }
        {
            let mut order_seq_state = order_seq_state.lock().await;
            attach_order_seq(
                &mut order_seq_state,
                &event_type,
                &mut payload,
                Some(&turn_id),
                assistant_sequence,
            );
        }
        let event = match append_session_event_with_retry(
            &store,
            session_id,
            Some(run_id),
            Some(turn_id),
            event_type.clone(),
            payload,
        )
        .await
        {
            Ok(event) => event,
            Err(err) => {
                tracing::warn!(
                    session_id = %session_id.0,
                    run_id = %run_id.0,
                    turn_id = %turn_id.0,
                    event_type = ?event_type,
                    "failed to append session event: {err:#}"
                );
                continue;
            }
        };
        let publish_after_persist = matches!(
            event.event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        );
        if !publish_after_persist {
            state.publish_event(event.clone()).await;
        }

        if start_progress == TurnStartProgress::Pending
            && is_truthful_start_activity(&event.event_type)
        {
            start_progress = TurnStartProgress::Started;
            let _ = start_progress_tx.send(TurnStartProgress::Started);
            let _ = store
                .update_session_turn_status(
                    session_id,
                    turn_id,
                    SessionTurnStatus::Running,
                    None,
                    None,
                    event.created_at,
                )
                .await;
        }

        match event.event_type {
            SessionEventType::AssistantChunk => {
                if let Some(fragment) = raw_payload.get("content_fragment").and_then(Value::as_str)
                {
                    assistant_partial.push_str(fragment);
                    if let Some(message_id) = raw_payload
                        .get("message_id")
                        .and_then(Value::as_str)
                        .map(|value| value.to_string())
                    {
                        assistant_partial_message_id = Some(message_id);
                    }
                }
            }
            SessionEventType::ThoughtChunk if should_track_thought_chunk(&raw_payload) => {
                if let Some(fragment) = raw_payload.get("content_fragment").and_then(Value::as_str)
                {
                    thought_partial.push_str(fragment);
                    let _ = store
                        .update_session_turn_partial(
                            session_id,
                            turn_id,
                            None,
                            Some(&thought_partial),
                            event.created_at,
                        )
                        .await;
                }
            }
            SessionEventType::Notice
                if event
                    .payload_json
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "session_gap") =>
            {
                let reason = event
                    .payload_json
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(|value| value.to_string());
                state
                    .workspaces
                    .workspace_active_snapshot
                    .publish_session_gap(workspace_id, session_id, event.seq, reason)
                    .await;
            }
            SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult => {
                if let Some(tool_event) = normalized_tool_event.as_ref() {
                    let order_seq = read_order_seq(&event.payload_json);
                    if let Some(update) = build_turn_tool_update(tool_event, order_seq) {
                        let prev = if matches!(event.event_type, SessionEventType::ToolCallUpdate) {
                            tool_cache.get(&update.tool_call_id).cloned()
                        } else if let Some(cached) = tool_cache.get(&update.tool_call_id).cloned() {
                            Some(cached)
                        } else {
                            store
                                .get_session_turn_tool(session_id, &update.tool_call_id)
                                .await
                                .ok()
                                .flatten()
                        };
                        if let Some(merged) = merge_tool_update(
                            prev.as_ref(),
                            update,
                            session_id,
                            turn_id,
                            event.seq,
                            event.created_at,
                        ) {
                            if matches!(event.event_type, SessionEventType::ToolCallUpdate) {
                                tool_cache.insert(merged.tool_call_id.clone(), merged);
                            } else {
                                let (
                                    delta_total,
                                    delta_pending,
                                    delta_running,
                                    delta_completed,
                                    delta_failed,
                                ) = tool_count_deltas(prev.as_ref(), &merged);
                                let _ = store.upsert_session_turn_tool(merged.clone()).await;
                                if delta_total != 0
                                    || delta_pending != 0
                                    || delta_running != 0
                                    || delta_completed != 0
                                    || delta_failed != 0
                                {
                                    let _ = store
                                        .update_session_turn_tool_counts(
                                            session_id,
                                            turn_id,
                                            SessionTurnToolCountDeltas {
                                                total: delta_total,
                                                pending: delta_pending,
                                                running: delta_running,
                                                completed: delta_completed,
                                                failed: delta_failed,
                                            },
                                            event.created_at,
                                        )
                                        .await;
                                }
                                tool_cache.insert(merged.tool_call_id.clone(), merged);
                            }
                        }
                    }
                }
            }
            SessionEventType::AssistantComplete => {
                let provider_message_id = event
                    .payload_json
                    .get("message_id")
                    .or_else(|| event.payload_json.get("messageId"))
                    .and_then(Value::as_str)
                    .map(|s: &str| s.to_string())
                    .or_else(|| assistant_partial_message_id.clone());
                let order_seq = read_order_seq(&event.payload_json).unwrap_or_else(|| {
                    tracing::warn!(
                        session_id = %session_id.0,
                        run_id = %run_id.0,
                        turn_id = %turn_id.0,
                        "assistant_complete missing order_seq; assigning fallback message order"
                    );
                    0
                });
                let content: Option<String> = event
                    .payload_json
                    .get("full_content")
                    .or_else(|| event.payload_json.get("content"))
                    .and_then(Value::as_str)
                    .map(|s: &str| s.to_string())
                    .or_else(|| {
                        if assistant_partial.is_empty() {
                            None
                        } else {
                            Some(assistant_partial.clone())
                        }
                    });
                if let Some(content) = content {
                    if let Some(content) = strip_emitted_prefix(&content, &assistant_emitted) {
                        let assistant_message_id = ctx_core::ids::MessageId::new();
                        let persisted_order_seq = if order_seq > 0 {
                            order_seq
                        } else {
                            let mut order_seq_state = order_seq_state.lock().await;
                            order_seq_state
                                .get_or_assign(format!("message:{}", assistant_message_id.0), None)
                        };
                        match persist_assistant_message(
                            state.as_ref(),
                            &store,
                            workspace_id,
                            assistant_message_id,
                            persisted_order_seq,
                            session_id,
                            task_id,
                            run_id,
                            turn_id,
                            content,
                            assistant_sequence + 1,
                            event.created_at,
                        )
                        .await
                        {
                            Ok(saved) => {
                                assistant_sequence += 1;
                                assistant_emitted.push_str(&saved.content);
                                assistant_partial.clear();
                                let mut payload = json!({
                                    "message_id": saved.id.0,
                                    "content": saved.content,
                                    "delivery": saved.delivery,
                                    "attachments": saved.attachments,
                                    "turn_sequence": saved.turn_sequence,
                                    "order_seq": saved.order_seq,
                                });
                                if let Some(provider_message_id) = provider_message_id {
                                    if let Some(obj) = payload.as_object_mut() {
                                        obj.insert(
                                            "provider_message_id".to_string(),
                                            json!(provider_message_id),
                                        );
                                    }
                                }
                                {
                                    let mut order_seq_state = order_seq_state.lock().await;
                                    attach_order_seq(
                                        &mut order_seq_state,
                                        &SessionEventType::AssistantMessageInserted,
                                        &mut payload,
                                        Some(&turn_id),
                                        assistant_sequence,
                                    );
                                }
                                let _ = emit_event(
                                    &state,
                                    session_id,
                                    Some(run_id),
                                    Some(turn_id),
                                    SessionEventType::AssistantMessageInserted,
                                    payload,
                                )
                                .await;
                                assistant_partial_message_id = None;
                            }
                            Err(err) => {
                                let err_string = format!("{err:#}");
                                let is_storage_exhausted =
                                    storage_guard::is_storage_exhaustion_error(&err_string);
                                let details = Some(json!({
                                    "provider_message_id": provider_message_id,
                                    "order_seq": persisted_order_seq,
                                    "turn_sequence": assistant_sequence + 1,
                                    "root_cause": err_string,
                                }));
                                let storage_status = state.storage_guard_snapshot();
                                fail_turn(
                                    TurnFailureContext {
                                        state: &state,
                                        session_id,
                                        worktree_id,
                                        run_id,
                                        turn_id,
                                        message_id,
                                        provider_id: &provider_id,
                                        model_id: &model_id,
                                        execution_environment_label: &execution_environment_label,
                                        session_root_kind: &session_root_kind,
                                        workdir_str: &workdir_str,
                                        run_started_at,
                                        perf_run_id: perf_run_id.as_ref(),
                                        telemetry_emitted: &mut telemetry_emitted,
                                        terminal_status: &mut terminal_status,
                                        emit_error_event: true,
                                    },
                                    TurnFailurePayload {
                                        error_message: if is_storage_exhausted {
                                            storage_guard::storage_exhaustion_message(
                                                storage_status.active.as_ref(),
                                            )
                                        } else {
                                            format!("failed to persist assistant message: {err:#}")
                                        },
                                        details,
                                        kind: Some(json!(if is_storage_exhausted {
                                            "storage_exhausted"
                                        } else {
                                            "assistant_message_persist_failed"
                                        })),
                                    },
                                )
                                .await;
                                assistant_partial.clear();
                                assistant_partial_message_id = None;
                            }
                        }
                    } else {
                        assistant_partial.clear();
                        assistant_partial_message_id = None;
                    }
                }
                let _ = store
                    .delete_session_events_for_turn_types(
                        session_id,
                        turn_id,
                        &[
                            SessionEventType::AssistantChunk,
                            SessionEventType::ThoughtChunk,
                        ],
                    )
                    .await;
            }
            SessionEventType::Done => {
                if start_progress != TurnStartProgress::Terminal {
                    start_progress = TurnStartProgress::Terminal;
                    let _ = start_progress_tx.send(TurnStartProgress::Terminal);
                }
                if terminal_status.is_some() {
                    continue;
                }
                if !telemetry_emitted {
                    telemetry_emitted = true;
                    let duration_ms = run_started_at.elapsed().as_millis() as u64;
                    let mut run_labels = HashMap::new();
                    run_labels.insert("provider_id".to_string(), provider_id.clone());
                    run_labels.insert("model_id".to_string(), model_id.clone());
                    run_labels.insert(
                        "execution_environment".to_string(),
                        execution_environment_label.clone(),
                    );
                    run_labels.insert("session_root_kind".to_string(), session_root_kind.clone());
                    run_labels.insert("event".to_string(), "run_complete".to_string());
                    let run_metric = PerfMetric {
                        name: "scheduler.run_total_ms".to_string(),
                        kind: PerfMetricKind::Histogram,
                        unit: "ms".to_string(),
                        value: duration_ms as f64,
                        labels: run_labels,
                    };
                    state
                        .telemetry
                        .perf_telemetry
                        .record_metric(run_metric, perf_run_id.clone(), None, None)
                        .await;
                    state
                        .telemetry
                        .telemetry
                        .emit(TelemetryEvent::provider_call(
                            provider_id.clone(),
                            model_id.clone(),
                            Some(execution_environment_label.clone()),
                            Some(session_root_kind.clone()),
                            true,
                            duration_ms,
                        ))
                        .await;
                    state
                        .telemetry
                        .telemetry
                        .emit(TelemetryEvent::session_completed(
                            provider_id.clone(),
                            model_id.clone(),
                            Some(execution_environment_label.clone()),
                            Some(session_root_kind.clone()),
                            "completed".to_string(),
                            duration_ms,
                        ))
                        .await;
                }
                let _ = store
                    .delete_session_events_for_turn_types(
                        session_id,
                        turn_id,
                        &[
                            SessionEventType::ThoughtChunk,
                            SessionEventType::ContextWindowUpdate,
                        ],
                    )
                    .await;
                terminal_status = Some(SessionTurnStatus::Completed);
            }
            SessionEventType::TurnInterrupted => {
                if start_progress != TurnStartProgress::Terminal {
                    start_progress = TurnStartProgress::Terminal;
                    let _ = start_progress_tx.send(TurnStartProgress::Terminal);
                }
                if terminal_status.is_some() {
                    continue;
                }
                if !telemetry_emitted {
                    telemetry_emitted = true;
                    let duration_ms = run_started_at.elapsed().as_millis() as u64;
                    let run_metric = PerfMetric {
                        name: "scheduler.run_total_ms".to_string(),
                        kind: PerfMetricKind::Histogram,
                        unit: "ms".to_string(),
                        value: duration_ms as f64,
                        labels: metric_labels(
                            &provider_id,
                            &model_id,
                            &execution_environment_label,
                            &session_root_kind,
                            "run_interrupt",
                        ),
                    };
                    state
                        .telemetry
                        .perf_telemetry
                        .record_metric(run_metric, perf_run_id.clone(), None, None)
                        .await;
                    state
                        .telemetry
                        .telemetry
                        .emit(TelemetryEvent::provider_call(
                            provider_id.clone(),
                            model_id.clone(),
                            Some(execution_environment_label.clone()),
                            Some(session_root_kind.clone()),
                            false,
                            duration_ms,
                        ))
                        .await;
                    state
                        .telemetry
                        .telemetry
                        .emit(TelemetryEvent::session_completed(
                            provider_id.clone(),
                            model_id.clone(),
                            Some(execution_environment_label.clone()),
                            Some(session_root_kind.clone()),
                            "interrupted".to_string(),
                            duration_ms,
                        ))
                        .await;
                }
                if let Some(requested_at_ms) = event
                    .payload_json
                    .get("requested_at_ms")
                    .and_then(Value::as_i64)
                {
                    let latency_ms =
                        (event.created_at.timestamp_millis() - requested_at_ms).max(0) as u64;
                    let bucket = latency_bucket(latency_ms).to_string();
                    let interrupt_metric = PerfMetric {
                        name: "scheduler.interrupt_total_ms".to_string(),
                        kind: PerfMetricKind::Histogram,
                        unit: "ms".to_string(),
                        value: latency_ms as f64,
                        labels: metric_labels(
                            &provider_id,
                            &model_id,
                            &execution_environment_label,
                            &session_root_kind,
                            "turn_interrupted_visible",
                        ),
                    };
                    state
                        .telemetry
                        .perf_telemetry
                        .record_metric(interrupt_metric, perf_run_id.clone(), None, None)
                        .await;
                    state
                        .telemetry
                        .telemetry
                        .emit(TelemetryEvent::session_interrupt_latency(
                            provider_id.clone(),
                            model_id.clone(),
                            Some(execution_environment_label.clone()),
                            Some(session_root_kind.clone()),
                            latency_ms,
                            bucket.clone(),
                        ))
                        .await;
                    let interrupt_id = event
                        .payload_json
                        .get("interrupt_id")
                        .and_then(serde_json::Value::as_str);
                    tracing::info!(
                        session_id = %session_id.0,
                        run_id = %run_id.0,
                        turn_id = %turn_id.0,
                        interrupt_id,
                        interrupt_total_ms = latency_ms,
                        duration_bucket = %bucket,
                        "session interrupt became visible in event loop"
                    );
                }
                terminal_status = Some(SessionTurnStatus::Interrupted);
                let _ = store
                    .delete_session_events_for_turn_types(
                        session_id,
                        turn_id,
                        &[
                            SessionEventType::AssistantChunk,
                            SessionEventType::ThoughtChunk,
                            SessionEventType::ContextWindowUpdate,
                        ],
                    )
                    .await;
            }
            SessionEventType::Error => {
                if start_progress != TurnStartProgress::Terminal {
                    start_progress = TurnStartProgress::Terminal;
                    let _ = start_progress_tx.send(TurnStartProgress::Terminal);
                }
                if terminal_status.is_some() {
                    continue;
                }
                let error_message = event
                    .payload_json
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("provider runtime error")
                    .to_string();
                record_failed_turn_telemetry(
                    &state,
                    session_id,
                    worktree_id,
                    run_id,
                    turn_id,
                    &provider_id,
                    &model_id,
                    &execution_environment_label,
                    &session_root_kind,
                    &workdir_str,
                    run_started_at,
                    perf_run_id.as_ref(),
                    &mut telemetry_emitted,
                    error_message,
                    event.payload_json.get("details").cloned(),
                    event.payload_json.get("kind").cloned(),
                )
                .await;
                terminal_status = Some(SessionTurnStatus::Failed);
                let _ = store
                    .delete_session_events_for_turn_types(
                        session_id,
                        turn_id,
                        &[
                            SessionEventType::AssistantChunk,
                            SessionEventType::ThoughtChunk,
                            SessionEventType::ContextWindowUpdate,
                        ],
                    )
                    .await;
            }
            _ => {}
        }
        if publish_after_persist {
            state.publish_event(event.clone()).await;
        }
    }
    let _ = events_done_tx.send(());
}

#[cfg(test)]
mod tests;
