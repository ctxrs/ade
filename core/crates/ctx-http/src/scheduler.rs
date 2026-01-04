use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionEvent, SessionEventType,
    SessionTurnStatus, SessionTurnTool,
};
use ctx_providers::adapters::{ProviderAdapter, RunHandle, TurnInput};
use ctx_providers::events::NormalizedEvent;
use ctx_store::store::SessionTurnToolCountDeltas;

use crate::daemon::AppState;
use crate::installer;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::telemetry::TelemetryEvent;

#[derive(Debug)]
pub struct QueuedMessage {
    pub message: Message,
    pub enqueued_at: Instant,
    pub run_id: Option<String>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SchedulerCommand {
    Enqueue(QueuedMessage),
    RemoveQueued(MessageId),
    Cancel,
    Interrupt,
}

struct RunningTurn {
    adapter: Arc<dyn ProviderAdapter>,
    handle: RunHandle,
    run_id: RunId,
    turn_id: TurnId,
    event_tx: mpsc::Sender<NormalizedEvent>,
}

pub async fn session_worker(
    state: Arc<AppState>,
    session: Session,
    mut rx: mpsc::Receiver<SchedulerCommand>,
) {
    let mut queue: VecDeque<QueuedMessage> = VecDeque::new();
    if let Ok(mut queued) = state
        .store
        .list_queued_messages_for_session(session.id)
        .await
    {
        for m in queued.drain(..) {
            queue.push_back(QueuedMessage {
                message: m,
                enqueued_at: Instant::now(),
                run_id: None,
            });
        }
    }
    let mut running: Option<RunningTurn> = None;
    let mut suspend_queue = false;

    let worktree = match state.store.get_worktree(session.worktree_id).await {
        Ok(Some(wt)) => wt,
        _ => return,
    };
    let workdir = PathBuf::from(worktree.root_path.clone());
    let env_target = if worktree.git_branch.is_some() {
        "worktree".to_string()
    } else {
        "local".to_string()
    };

    loop {
        if running.is_none() && !suspend_queue {
            if let Some(msg) = queue.pop_front() {
                match start_turn(&state, &session, &workdir, &env_target, msg).await {
                    Ok(turn) => {
                        state.set_running(session.id, true).await;
                        running = Some(turn);
                    }
                    Err(_) => {
                        state.set_running(session.id, false).await;
                        running = None;
                    }
                }
                continue;
            }
        }

        tokio::select! {
            cmd = rx.recv() => {
                match cmd {
                    Some(SchedulerCommand::Enqueue(msg)) => {
                        if running.is_some() {
                            queue.push_back(msg);
                        } else {
                            if matches!(msg.message.delivery, MessageDelivery::Immediate) {
                                suspend_queue = false;
                            }
                            queue.push_front(msg);
                        }
                    }
                    Some(SchedulerCommand::RemoveQueued(id)) => {
                        let mut next = VecDeque::new();
                        while let Some(m) = queue.pop_front() {
                            if m.message.id != id {
                                next.push_back(m);
                            }
                        }
                        queue = next;
                    }
                    Some(SchedulerCommand::Cancel) => {
                        if let Some(turn) = running.take() {
                            let _ = turn.adapter.cancel(turn.handle).await;
                            if !send_turn_interrupted(
                                &turn.event_tx,
                                "user_cancel",
                                true,
                            )
                            .await
                            {
                                let _ = reconcile_turn_terminal_state(
                                    &state,
                                    session.id,
                                    Some(turn.run_id),
                                    turn.turn_id,
                                    "user_cancel",
                                )
                                .await;
                            }
                            state.set_running(session.id, false).await;
                        }
                    }
                    Some(SchedulerCommand::Interrupt) => {
                        if let Some(turn) = running.take() {
                            let _ = emit_event(
                                &state,
                                session.id,
                                Some(turn.run_id),
                                Some(turn.turn_id),
                                SessionEventType::InterruptRequested,
                                json!({"by":"user"}),
                            ).await;
                            let _ = turn.adapter.cancel(turn.handle).await;
                            if !send_turn_interrupted(
                                &turn.event_tx,
                                "user_interrupt",
                                true,
                            )
                            .await
                            {
                                let _ = reconcile_turn_terminal_state(
                                    &state,
                                    session.id,
                                    Some(turn.run_id),
                                    turn.turn_id,
                                    "user_interrupt",
                                )
                                .await;
                            }
                            state.set_running(session.id, false).await;
                            suspend_queue = true;
                        }
                    }
                    None => break,
                }
            }
            _ = async {
                if let Some(turn) = running.as_mut() {
                    let _ = (&mut turn.handle.done).await;
                }
            }, if running.is_some() => {
                running = None;
                state.set_running(session.id, false).await;
            }
        }
    }
}

async fn start_turn(
    state: &Arc<AppState>,
    session: &Session,
    workdir: &Path,
    env_target: &str,
    queued: QueuedMessage,
) -> Result<RunningTurn> {
    state.wait_for_worktree_bootstrap(session.worktree_id).await;

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id)
            .cloned()
            .ok_or_else(|| anyhow!("provider not available: {}", session.provider_id))?
    };

    let mut message = queued.message;
    let perf_run_id = queued.run_id.clone();
    let queue_wait_ms = queued.enqueued_at.elapsed().as_millis() as u64;
    let mut queue_labels = HashMap::new();
    queue_labels.insert("provider_id".to_string(), session.provider_id.clone());
    queue_labels.insert("model_id".to_string(), session.model_id.clone());
    queue_labels.insert("env_target".to_string(), env_target.to_string());
    queue_labels.insert("event".to_string(), "queue_wait".to_string());
    let queue_metric = PerfMetric {
        name: "scheduler.queue_wait_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: queue_wait_ms as f64,
        labels: queue_labels,
    };
    state
        .perf_telemetry
        .record_metric(queue_metric, perf_run_id.clone(), None, None)
        .await;
    let run_id = message.run_id.get_or_insert_with(RunId::new).to_owned();
    let turn_id = message.turn_id.get_or_insert_with(TurnId::new).to_owned();

    if message.delivered_at.is_none() {
        state.store.mark_message_delivered(message.id).await?;
        message.delivery = MessageDelivery::Immediate;
        message.delivered_at = Some(Utc::now());
    }
    let _ = state
        .store
        .update_session_turn_status(
            session.id,
            turn_id,
            SessionTurnStatus::Running,
            None,
            None,
            Utc::now(),
        )
        .await;

    let prompt = message.content.clone();
    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, &session.model_id, &prompt);

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let event_tx = ev_tx.clone();

    let mut provider_env = std::collections::HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.data_root.to_string_lossy().to_string(),
    );
    if let Some(token) = state.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    let mcp_token = uuid::Uuid::new_v4().to_string();
    provider_env.insert("CTX_MCP_TOKEN".to_string(), mcp_token);
    if let Ok(v) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CTX_MCP_DISABLED") {
        provider_env.insert("CTX_MCP_DISABLED".to_string(), v);
    }

    if let Ok(cfg) = installer::load_agent_server_config(&state.data_root).await {
        if let Some(cmd) = cfg.providers.get(&session.provider_id) {
            let mut bin_dirs: Vec<std::path::PathBuf> = Vec::new();
            for dep in &cmd.dependencies {
                if let Some(meta) = cfg.managed_installs.get(dep) {
                    if let Some(rel) = meta.bin_dir_rel.as_ref() {
                        bin_dirs.push(state.data_root.join(rel));
                    }
                }
            }
            if !bin_dirs.is_empty() {
                let mut path_parts: Vec<std::path::PathBuf> = bin_dirs;
                if let Some(current) = std::env::var_os("PATH") {
                    path_parts.extend(std::env::split_paths(&current));
                }
                if let Ok(joined) = std::env::join_paths(path_parts) {
                    provider_env.insert("PATH".to_string(), joined.to_string_lossy().to_string());
                }
            }
        }
    }

    let run_started_at = Instant::now();
    let spawn_started_at = Instant::now();
    let handle = match adapter
        .run(
            TurnInput {
                content: prompt,
                attachments: message.attachments.clone(),
                context_blocks: Vec::new(),
            },
            workdir.to_path_buf(),
            provider_env,
            ev_tx,
        )
        .await
    {
        Ok(handle) => {
            let spawn_ms = spawn_started_at.elapsed().as_millis() as u64;
            let mut spawn_labels = HashMap::new();
            spawn_labels.insert("provider_id".to_string(), session.provider_id.clone());
            spawn_labels.insert("model_id".to_string(), session.model_id.clone());
            spawn_labels.insert("env_target".to_string(), env_target.to_string());
            spawn_labels.insert("event".to_string(), "spawn".to_string());
            let spawn_metric = PerfMetric {
                name: "provider.spawn_ms".to_string(),
                kind: PerfMetricKind::Histogram,
                unit: "ms".to_string(),
                value: spawn_ms as f64,
                labels: spawn_labels,
            };
            state
                .perf_telemetry
                .record_metric(spawn_metric, perf_run_id.clone(), None, None)
                .await;
            handle
        }
        Err(err) => {
            let duration_ms = run_started_at.elapsed().as_millis() as u64;
            state
                .telemetry
                .emit(TelemetryEvent::provider_call(
                    session.provider_id.clone(),
                    session.model_id.clone(),
                    Some(env_target.to_string()),
                    false,
                    duration_ms,
                ))
                .await;
            return Err(err);
        }
    };

    let state_for_events = Arc::clone(state);
    let store = state.store.clone();
    let session_id = session.id;
    let task_id = session.task_id;
    let track_id = session.track_id;
    let provider_id = session.provider_id.clone();
    let model_id = session.model_id.clone();
    let env_target = env_target.to_string();
    let perf_run_id = perf_run_id.clone();
    let mut telemetry_emitted = false;

    tokio::spawn(async move {
        let mut assistant_partial = String::new();
        let mut assistant_sequence: i64 = 0;
        let mut thought_partial = String::new();
        let mut tool_cache: HashMap<String, SessionTurnTool> = HashMap::new();
        let mut terminal_status: Option<SessionTurnStatus> = None;
        let mut first_event_at: Option<Instant> = None;

        while let Some(ev) = ev_rx.recv().await {
            let event_type = ev.event_type.clone();
            let raw_payload = ev.payload_json.clone();
            let mut payload = raw_payload.clone();
            if first_event_at.is_none() {
                first_event_at = Some(Instant::now());
                let first_ms = run_started_at.elapsed().as_millis() as u64;
                let mut first_labels = HashMap::new();
                first_labels.insert("provider_id".to_string(), provider_id.clone());
                first_labels.insert("model_id".to_string(), model_id.clone());
                first_labels.insert("env_target".to_string(), env_target.clone());
                first_labels.insert("event".to_string(), "first_event".to_string());
                let first_metric = PerfMetric {
                    name: "provider.first_event_ms".to_string(),
                    kind: PerfMetricKind::Histogram,
                    unit: "ms".to_string(),
                    value: first_ms as f64,
                    labels: first_labels,
                };
                state_for_events
                    .perf_telemetry
                    .record_metric(first_metric, perf_run_id.clone(), None, None)
                    .await;
            }
            if matches!(ev.event_type, SessionEventType::Init) {
                if let Some(ps) = payload.get("acp_session_id").and_then(Value::as_str) {
                    let _ = store
                        .update_session_provider_session_ref(session_id, Some(ps.to_string()))
                        .await;
                }
            }
            if matches!(ev.event_type, SessionEventType::Done) {
                if let (Some(metrics), Some(obj)) =
                    (context_window_metrics.clone(), payload.as_object_mut())
                {
                    obj.entry("context_window").or_insert(metrics);
                    obj.entry("status").or_insert(json!("completed"));
                }
            }
            if matches!(
                event_type,
                SessionEventType::ToolCall
                    | SessionEventType::ToolCallUpdate
                    | SessionEventType::ToolResult
            ) {
                payload = sanitize_tool_event_payload(&event_type, &raw_payload);
            }
            let appended = store
                .append_session_event(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    event_type.clone(),
                    payload,
                )
                .await;
            if let Ok(event) = appended {
                state_for_events.publish_event(event.clone()).await;

                match event.event_type {
                    SessionEventType::AssistantChunk => {
                        if let Some(fragment) =
                            raw_payload.get("content_fragment").and_then(Value::as_str)
                        {
                            assistant_partial.push_str(fragment);
                            let _ = store
                                .update_session_turn_partial(
                                    session_id,
                                    turn_id,
                                    Some(&assistant_partial),
                                    None,
                                    event.created_at,
                                )
                                .await;
                        }
                    }
                    SessionEventType::ThoughtChunk => {
                        if should_track_thought_chunk(&raw_payload) {
                            if let Some(fragment) =
                                raw_payload.get("content_fragment").and_then(Value::as_str)
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
                    }
                    SessionEventType::ToolCall
                    | SessionEventType::ToolCallUpdate
                    | SessionEventType::ToolResult => {
                        if !assistant_partial.is_empty() {
                            if let Ok(saved) = persist_assistant_message(
                                &store,
                                session_id,
                                task_id,
                                track_id,
                                run_id,
                                turn_id,
                                assistant_partial.clone(),
                                assistant_sequence + 1,
                                event.created_at,
                            )
                            .await
                            {
                                assistant_sequence += 1;
                                assistant_partial.clear();
                                let _ = store
                                    .update_session_turn_partial(
                                        session_id,
                                        turn_id,
                                        Some(""),
                                        None,
                                        event.created_at,
                                    )
                                    .await;
                                let _ = emit_event(
                                    &state_for_events,
                                    session_id,
                                    Some(run_id),
                                    Some(turn_id),
                                    SessionEventType::AssistantMessageInserted,
                                    json!({
                                        "message_id": saved.id.0,
                                        "turn_sequence": saved.turn_sequence,
                                    }),
                                )
                                .await;
                            }
                        }
                        if let Some(update) =
                            build_turn_tool_update_from_payload(&event_type, &raw_payload)
                        {
                            let prev = if let Some(cached) =
                                tool_cache.get(&update.tool_call_id).cloned()
                            {
                                Some(cached)
                            } else {
                                store
                                    .get_session_turn_tool(session_id, &update.tool_call_id)
                                    .await
                                    .ok()
                                    .flatten()
                            };
                            let merged = merge_tool_update(
                                prev.as_ref(),
                                update,
                                session_id,
                                turn_id,
                                event.created_at,
                            );
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
                    SessionEventType::AssistantComplete => {
                        let content = event
                            .payload_json
                            .get("full_content")
                            .or_else(|| event.payload_json.get("content"))
                            .and_then(Value::as_str)
                            .map(|s| s.to_string())
                            .or_else(|| {
                                if assistant_partial.is_empty() {
                                    None
                                } else {
                                    Some(assistant_partial.clone())
                                }
                            });
                        if let Some(content) = content {
                            if !content.is_empty() {
                                if let Ok(saved) = persist_assistant_message(
                                    &store,
                                    session_id,
                                    task_id,
                                    track_id,
                                    run_id,
                                    turn_id,
                                    content,
                                    assistant_sequence + 1,
                                    event.created_at,
                                )
                                .await
                                {
                                    assistant_sequence += 1;
                                    assistant_partial.clear();
                                    let _ = store
                                        .update_session_turn_partial(
                                            session_id,
                                            turn_id,
                                            Some(""),
                                            None,
                                            event.created_at,
                                        )
                                        .await;
                                    let _ = emit_event(
                                        &state_for_events,
                                        session_id,
                                        Some(run_id),
                                        Some(turn_id),
                                        SessionEventType::AssistantMessageInserted,
                                        json!({
                                            "message_id": saved.id.0,
                                            "turn_sequence": saved.turn_sequence,
                                        }),
                                    )
                                    .await;
                                }
                            }
                        }
                        if terminal_status.is_none() {
                            let _ = store
                                .update_session_turn_status(
                                    session_id,
                                    turn_id,
                                    SessionTurnStatus::Completed,
                                    None,
                                    None,
                                    event.created_at,
                                )
                                .await;
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
                        if !telemetry_emitted {
                            telemetry_emitted = true;
                            let duration_ms = run_started_at.elapsed().as_millis() as u64;
                            let mut run_labels = HashMap::new();
                            run_labels.insert("provider_id".to_string(), provider_id.clone());
                            run_labels.insert("model_id".to_string(), model_id.clone());
                            run_labels.insert("env_target".to_string(), env_target.clone());
                            run_labels.insert("event".to_string(), "run_complete".to_string());
                            let run_metric = PerfMetric {
                                name: "scheduler.run_total_ms".to_string(),
                                kind: PerfMetricKind::Histogram,
                                unit: "ms".to_string(),
                                value: duration_ms as f64,
                                labels: run_labels,
                            };
                            state_for_events
                                .perf_telemetry
                                .record_metric(run_metric, perf_run_id.clone(), None, None)
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::provider_call(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    true,
                                    duration_ms,
                                ))
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::session_completed(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    "completed".to_string(),
                                    duration_ms,
                                ))
                                .await;
                        }
                        let metrics = event.payload_json.get("context_window");
                        if terminal_status.is_none() {
                            let _ = store
                                .update_session_turn_status(
                                    session_id,
                                    turn_id,
                                    SessionTurnStatus::Completed,
                                    Some(event.seq),
                                    metrics,
                                    event.created_at,
                                )
                                .await;
                        }
                        let _ = store
                            .delete_session_events_for_turn_types(
                                session_id,
                                turn_id,
                                &[SessionEventType::ThoughtChunk],
                            )
                            .await;
                    }
                    SessionEventType::TurnInterrupted => {
                        if !telemetry_emitted {
                            telemetry_emitted = true;
                            let duration_ms = run_started_at.elapsed().as_millis() as u64;
                            let mut run_labels = HashMap::new();
                            run_labels.insert("provider_id".to_string(), provider_id.clone());
                            run_labels.insert("model_id".to_string(), model_id.clone());
                            run_labels.insert("env_target".to_string(), env_target.clone());
                            run_labels.insert("event".to_string(), "run_interrupt".to_string());
                            let run_metric = PerfMetric {
                                name: "scheduler.run_total_ms".to_string(),
                                kind: PerfMetricKind::Histogram,
                                unit: "ms".to_string(),
                                value: duration_ms as f64,
                                labels: run_labels,
                            };
                            state_for_events
                                .perf_telemetry
                                .record_metric(run_metric, perf_run_id.clone(), None, None)
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::provider_call(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    false,
                                    duration_ms,
                                ))
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::session_completed(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    "interrupted".to_string(),
                                    duration_ms,
                                ))
                                .await;
                        }
                        terminal_status = Some(SessionTurnStatus::Interrupted);
                        let _ = store
                            .update_session_turn_status(
                                session_id,
                                turn_id,
                                SessionTurnStatus::Interrupted,
                                Some(event.seq),
                                None,
                                event.created_at,
                            )
                            .await;
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
                    SessionEventType::Error => {
                        if !telemetry_emitted {
                            telemetry_emitted = true;
                            let duration_ms = run_started_at.elapsed().as_millis() as u64;
                            let mut run_labels = HashMap::new();
                            run_labels.insert("provider_id".to_string(), provider_id.clone());
                            run_labels.insert("model_id".to_string(), model_id.clone());
                            run_labels.insert("env_target".to_string(), env_target.clone());
                            run_labels.insert("event".to_string(), "run_failed".to_string());
                            let run_metric = PerfMetric {
                                name: "scheduler.run_total_ms".to_string(),
                                kind: PerfMetricKind::Histogram,
                                unit: "ms".to_string(),
                                value: duration_ms as f64,
                                labels: run_labels,
                            };
                            state_for_events
                                .perf_telemetry
                                .record_metric(run_metric, perf_run_id.clone(), None, None)
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::provider_call(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    false,
                                    duration_ms,
                                ))
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::session_completed(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    "failed".to_string(),
                                    duration_ms,
                                ))
                                .await;
                        }
                        terminal_status = Some(SessionTurnStatus::Failed);
                        let _ = store
                            .update_session_turn_status(
                                session_id,
                                turn_id,
                                SessionTurnStatus::Failed,
                                None,
                                None,
                                event.created_at,
                            )
                            .await;
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
                    _ => {}
                }
            }
        }
    });

    Ok(RunningTurn {
        adapter,
        handle,
        run_id,
        turn_id,
        event_tx,
    })
}

async fn send_turn_interrupted(
    event_tx: &mpsc::Sender<NormalizedEvent>,
    reason: &str,
    provider_cancelled: bool,
) -> bool {
    event_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::TurnInterrupted,
            payload_json: json!({
                "reason": reason,
                "provider_cancelled": provider_cancelled
            }),
        })
        .await
        .is_ok()
}

pub async fn reconcile_turn_terminal_state(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    fallback_reason: &str,
) -> Result<()> {
    let turn = state.store.get_session_turn(session_id, turn_id).await?;
    let Some(turn) = turn else {
        return Ok(());
    };
    if matches!(
        turn.status,
        SessionTurnStatus::Completed | SessionTurnStatus::Failed | SessionTurnStatus::Interrupted
    ) {
        return Ok(());
    }

    let events = state
        .store
        .list_session_events_for_turn(session_id, turn_id)
        .await?;
    if let Some(event) = events.iter().rev().find(|ev| {
        matches!(
            ev.event_type,
            SessionEventType::Done | SessionEventType::Error | SessionEventType::TurnInterrupted
        )
    }) {
        match event.event_type {
            SessionEventType::Done => {
                let metrics = event.payload_json.get("context_window");
                let _ = state
                    .store
                    .update_session_turn_status(
                        session_id,
                        turn_id,
                        SessionTurnStatus::Completed,
                        Some(event.seq),
                        metrics,
                        event.created_at,
                    )
                    .await;
            }
            SessionEventType::TurnInterrupted => {
                let _ = state
                    .store
                    .update_session_turn_status(
                        session_id,
                        turn_id,
                        SessionTurnStatus::Interrupted,
                        Some(event.seq),
                        None,
                        event.created_at,
                    )
                    .await;
            }
            SessionEventType::Error => {
                let _ = state
                    .store
                    .update_session_turn_status(
                        session_id,
                        turn_id,
                        SessionTurnStatus::Failed,
                        None,
                        None,
                        event.created_at,
                    )
                    .await;
            }
            _ => {}
        }
        return Ok(());
    }

    let event = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::TurnInterrupted,
        json!({"reason": fallback_reason, "provider_cancelled": false}),
    )
    .await?;
    let _ = state
        .store
        .update_session_turn_status(
            session_id,
            turn_id,
            SessionTurnStatus::Interrupted,
            Some(event.seq),
            None,
            event.created_at,
        )
        .await;
    Ok(())
}

#[allow(dead_code)]
async fn build_rehydrate_transcript_block(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
) -> Result<serde_json::Value> {
    let msgs = store.list_messages_for_session(session_id).await?;
    if msgs.is_empty() {
        anyhow::bail!("no messages to rehydrate");
    }

    const MAX_MESSAGES: usize = 24;
    const MAX_CHARS_PER_MESSAGE: usize = 4000;
    let tail: Vec<_> = msgs
        .into_iter()
        .rev()
        .take(MAX_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut text = String::new();
    text.push_str("Session transcript (for continuity after ctx daemon restart):\n\n");
    for m in tail {
        let role = match m.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        let mut content = m.content;
        if content.chars().count() > MAX_CHARS_PER_MESSAGE {
            content = content
                .chars()
                .take(MAX_CHARS_PER_MESSAGE)
                .collect::<String>();
            content.push_str("\n…(truncated)");
        }
        text.push_str(&format!(
            "[{}] {role}:\n{content}\n\n",
            m.created_at.to_rfc3339()
        ));
    }

    Ok(json!({
        "type": "resource",
        "resource": {
            "uri": format!("ctx://session/{}/transcript", session_id.0),
            "mimeType": "text/plain",
            "text": text
        }
    }))
}
async fn emit_event(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    event_type: SessionEventType,
    payload_json: serde_json::Value,
) -> Result<SessionEvent> {
    let event = state
        .store
        .append_session_event(session_id, run_id, turn_id, event_type, payload_json)
        .await?;
    state.publish_event(event.clone()).await;
    Ok(event)
}

fn should_track_thought_chunk(payload: &serde_json::Value) -> bool {
    let meta = payload
        .get("acp_update")
        .and_then(|v| v.get("_meta"))
        .or_else(|| payload.get("acp_update").and_then(|v| v.get("meta")))
        .or_else(|| payload.get("_meta"))
        .or_else(|| payload.get("meta"));
    if meta
        .and_then(|v| v.get("heartbeat"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    let has_status_text = |meta: &Value| {
        for key in ["status_text", "statusText", "status_string", "statusString"] {
            if meta.get(key).and_then(Value::as_str).is_some() {
                return true;
            }
        }
        if let Some(status) = meta.get("status").and_then(Value::as_str) {
            let s = status.trim().to_lowercase();
            let is_tool_status = matches!(
                s.as_str(),
                "pending"
                    | "queued"
                    | "running"
                    | "in_progress"
                    | "completed"
                    | "failed"
                    | "error"
                    | "ok"
                    | "success"
                    | "succeeded"
            );
            if !is_tool_status {
                return true;
            }
        }
        false
    };

    let reasoning_kind = meta
        .and_then(|v| v.get("codex"))
        .and_then(|v| v.get("reasoning_kind").or_else(|| v.get("reasoningKind")))
        .and_then(Value::as_str);
    if matches!(reasoning_kind, Some("summary" | "status")) {
        return false;
    }
    if let Some(meta) = meta {
        if has_status_text(meta) {
            return false;
        }
        if let Some(codex_meta) = meta.get("codex") {
            if has_status_text(codex_meta) {
                return false;
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
async fn persist_assistant_message(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
    task_id: ctx_core::ids::TaskId,
    track_id: ctx_core::ids::TrackId,
    run_id: RunId,
    turn_id: TurnId,
    content: String,
    turn_sequence: i64,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<Message> {
    if content.is_empty() {
        return Err(anyhow!("assistant message content empty"));
    }
    let msg = Message {
        id: ctx_core::ids::MessageId::new(),
        session_id,
        task_id,
        track_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(turn_sequence),
        role: MessageRole::Assistant,
        content,
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: Some(created_at),
        created_at,
    };
    store.insert_message(msg).await
}

fn compute_context_window_metrics(
    provider_id: &str,
    model_id: &str,
    prompt: &str,
) -> Option<serde_json::Value> {
    let context_window_tokens = model_context_window(provider_id, model_id)?;
    let context_tokens_estimate = estimate_tokens(prompt);
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(context_tokens_estimate);
    let remaining_fraction = remaining_tokens_estimate as f64 / context_window_tokens as f64;
    Some(json!({
        "context_tokens_estimate": context_tokens_estimate,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
    }))
}

fn model_context_window(provider_id: &str, model_id: &str) -> Option<usize> {
    match (provider_id, model_id) {
        ("fake", "fake-model") => Some(8192),
        _ => None,
    }
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.div_ceil(4)
}

#[derive(Default)]
struct DiffStats {
    added: usize,
    removed: usize,
    files: usize,
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn diff_stats_from_patch(patch: &str) -> Option<DiffStats> {
    let mut stats = DiffStats::default();
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            stats.files += 1;
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            continue;
        }
        if line.starts_with('+') {
            stats.added += 1;
            continue;
        }
        if line.starts_with('-') {
            stats.removed += 1;
        }
    }
    if stats.files == 0 && (stats.added > 0 || stats.removed > 0) {
        stats.files = 1;
    }
    if stats.files == 0 && stats.added == 0 && stats.removed == 0 {
        None
    } else {
        Some(stats)
    }
}

fn diff_stats_from_edits(edits: &[Value]) -> Option<DiffStats> {
    let mut stats = DiffStats::default();
    let mut files = HashSet::new();
    for edit in edits {
        let Some(obj) = edit.as_object() else {
            continue;
        };
        for key in [
            "path",
            "file",
            "file_path",
            "filePath",
            "filepath",
            "target",
        ] {
            if let Some(Value::String(path)) = obj.get(key) {
                files.insert(path.clone());
            }
        }
        let old_text = obj
            .get("oldText")
            .or_else(|| obj.get("old_text"))
            .or_else(|| obj.get("old"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_text = obj
            .get("newText")
            .or_else(|| obj.get("new_text"))
            .or_else(|| obj.get("new"))
            .or_else(|| obj.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        stats.removed += count_lines(old_text);
        stats.added += count_lines(new_text);
    }
    stats.files = files.len();
    if stats.files == 0 && (stats.added > 0 || stats.removed > 0) {
        stats.files = 1;
    }
    if stats.files == 0 && stats.added == 0 && stats.removed == 0 {
        None
    } else {
        Some(stats)
    }
}

fn extract_patch_text(input: &Value) -> Option<&str> {
    input
        .get("patch")
        .or_else(|| input.get("diff"))
        .or_else(|| input.get("patch_text"))
        .or_else(|| input.get("unified_diff"))
        .and_then(|v| v.as_str())
}

fn is_edit_tool(tool_kind: Option<&str>, title: Option<&str>) -> bool {
    let kind = tool_kind.unwrap_or("").trim().to_lowercase();
    if matches!(kind.as_str(), "edit" | "write" | "apply_patch" | "patch") {
        return true;
    }
    let title = title.unwrap_or("").trim().to_lowercase();
    title.contains("edit") || title.contains("patch") || title.contains("apply")
}

fn tool_input_preview(
    input: Option<&Value>,
    tool_kind: Option<&str>,
    title: Option<&str>,
) -> Option<Value> {
    let input = input?;
    let obj = input.as_object()?;
    let mut out = serde_json::Map::new();
    for key in [
        "command",
        "query",
        "pattern",
        "text",
        "path",
        "file",
        "filename",
        "file_path",
        "filePath",
        "filepath",
        "target",
        "paths",
        "files",
        "file_paths",
        "glob",
        "parsed_cmd",
    ] {
        if let Some(value) = obj.get(key) {
            if value.is_string() || value.is_array() || value.is_object() {
                out.insert(key.to_string(), value.clone());
            }
        }
    }

    if is_edit_tool(tool_kind, title) {
        let stats = extract_patch_text(input)
            .and_then(diff_stats_from_patch)
            .or_else(|| {
                obj.get("edits")
                    .and_then(|v| v.as_array())
                    .and_then(|edits| diff_stats_from_edits(edits))
            });
        if let Some(stats) = stats {
            out.insert(
                "diff_stats".to_string(),
                json!({
                    "added": stats.added,
                    "removed": stats.removed,
                    "files": stats.files,
                }),
            );
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

fn is_shell_like_tool(tool_kind: Option<&str>, title: Option<&str>) -> bool {
    let kind = tool_kind.unwrap_or("").trim().to_lowercase();
    if matches!(
        kind.as_str(),
        "shell" | "bash" | "sh" | "command" | "terminal"
    ) {
        return true;
    }
    let title = title.unwrap_or("").trim().to_lowercase();
    title.contains("shell") || title.contains("bash") || title.contains("terminal")
}

fn build_output_preview(text: &str, head_tail_lines: usize, max_chars: usize) -> (String, bool) {
    if head_tail_lines == 0 {
        return (String::new(), !text.trim().is_empty());
    }

    let mut total_lines: usize = 0;
    let mut first: Vec<String> = Vec::with_capacity(head_tail_lines);
    let mut last: VecDeque<String> = VecDeque::with_capacity(head_tail_lines);
    let mut first_2n_plus_1: Vec<String> = Vec::with_capacity(head_tail_lines * 2 + 1);

    for line in text.lines() {
        total_lines += 1;
        if first.len() < head_tail_lines {
            first.push(line.to_string());
        }
        if first_2n_plus_1.len() < head_tail_lines * 2 + 1 {
            first_2n_plus_1.push(line.to_string());
        }
        last.push_back(line.to_string());
        if last.len() > head_tail_lines {
            let _ = last.pop_front();
        }
    }

    if total_lines == 0 {
        return (String::new(), false);
    }

    let mut out = if total_lines <= head_tail_lines * 2 {
        first_2n_plus_1.join(
            "
",
        )
    } else {
        let omitted = total_lines.saturating_sub(head_tail_lines * 2);
        let mut parts: Vec<String> = Vec::with_capacity(head_tail_lines * 2 + 1);
        parts.extend(first);
        parts.push(format!("... +{omitted} lines"));
        parts.extend(last);
        parts.join(
            "
",
        )
    };

    let truncated = out.chars().count() > max_chars;
    if truncated {
        out = out.chars().take(max_chars).collect::<String>();
        out.push_str(
            "
... [preview truncated]",
        );
    }
    (out, truncated)
}

fn sanitize_tool_event_payload(event_type: &SessionEventType, raw_payload: &Value) -> Value {
    let update = extract_tool_update(raw_payload);
    let tool_call_id = tool_call_id_from_payload(raw_payload).unwrap_or_default();

    let tool_kind = update
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/kind").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let title = update
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/title").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/name").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    let raw_status = update
        .get("status")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));
    let status = if let Some(raw) = raw_status {
        normalize_tool_status(raw, event_type.clone())
    } else if matches!(event_type, SessionEventType::ToolResult) {
        "completed".to_string()
    } else {
        "pending".to_string()
    };

    let input = update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"));
    let input_preview = tool_input_preview(input, tool_kind.as_deref(), title.as_deref());

    let patch_preview = if is_edit_tool(tool_kind.as_deref(), title.as_deref()) {
        input.and_then(|v| extract_patch_text(v)).and_then(|t| {
            let (preview, _truncated) = build_output_preview(t, 5, 16_384);
            if preview.trim().is_empty() {
                None
            } else {
                Some(preview)
            }
        })
    } else {
        None
    };

    let output_preview = extract_tool_output_text(update)
        .and_then(|t| {
            let shell_like = is_shell_like_tool(tool_kind.as_deref(), title.as_deref());
            let lines = if shell_like { 50 } else { 5 };
            let (preview, _truncated) = build_output_preview(&t, lines, 16_384);
            if preview.trim().is_empty() {
                None
            } else {
                Some(preview)
            }
        })
        .or(patch_preview);

    let mut obj = serde_json::Map::new();
    if !tool_call_id.trim().is_empty() {
        obj.insert("tool_call_id".to_string(), Value::String(tool_call_id));
    }
    if let Some(v) = tool_kind {
        obj.insert("kind".to_string(), Value::String(v));
    }
    if let Some(v) = title {
        obj.insert("title".to_string(), Value::String(v));
    }
    obj.insert("status".to_string(), Value::String(status));
    if let Some(v) = input_preview {
        obj.insert("input_preview".to_string(), v);
    }
    if let Some(v) = output_preview {
        obj.insert("output_preview".to_string(), Value::String(v));
    }
    Value::Object(obj)
}

#[derive(Clone, Debug)]
struct TurnToolUpdate {
    tool_call_id: String,
    tool_kind: Option<String>,
    title: Option<String>,
    status: Option<String>,
    input_json: Option<Value>,
    output_text: Option<String>,
}

fn build_turn_tool_update_from_payload(
    event_type: &SessionEventType,
    payload_json: &Value,
) -> Option<TurnToolUpdate> {
    let update = extract_tool_update(payload_json);
    let tool_call_id = tool_call_id_from_payload(payload_json)?;
    let tool_kind = update
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/kind").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let title = update
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/title").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/name").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let raw_status = update
        .get("status")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));
    let status = if let Some(raw) = raw_status {
        Some(normalize_tool_status(raw, event_type.clone()))
    } else if matches!(event_type, SessionEventType::ToolResult) {
        Some("completed".to_string())
    } else if matches!(event_type, SessionEventType::ToolCall) {
        Some("pending".to_string())
    } else {
        None
    };

    let raw_input = update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"))
        .cloned();
    let input_json = tool_input_preview(raw_input.as_ref(), tool_kind.as_deref(), title.as_deref());

    let patch_preview = if is_edit_tool(tool_kind.as_deref(), title.as_deref()) {
        raw_input
            .as_ref()
            .and_then(|v| extract_patch_text(v))
            .and_then(|t| {
                let (preview, _truncated) = build_output_preview(t, 5, 16_384);
                if preview.trim().is_empty() {
                    None
                } else {
                    Some(preview)
                }
            })
    } else {
        None
    };

    let output_text = extract_tool_output_text(update)
        .and_then(|t| {
            let shell_like = is_shell_like_tool(tool_kind.as_deref(), title.as_deref());
            let lines = if shell_like { 50 } else { 5 };
            let (preview, _truncated) = build_output_preview(&t, lines, 16_384);
            if preview.trim().is_empty() {
                None
            } else {
                Some(preview)
            }
        })
        .or(patch_preview);

    Some(TurnToolUpdate {
        tool_call_id,
        tool_kind,
        title,
        status,
        input_json,
        output_text,
    })
}

fn merge_tool_update(
    prev: Option<&SessionTurnTool>,
    update: TurnToolUpdate,
    session_id: ctx_core::ids::SessionId,
    turn_id: ctx_core::ids::TurnId,
    now: chrono::DateTime<chrono::Utc>,
) -> SessionTurnTool {
    let created_at = prev.map(|t| t.created_at).unwrap_or(now);
    let tool_kind = update
        .tool_kind
        .or_else(|| prev.and_then(|t| t.tool_kind.clone()));
    let title = update.title.or_else(|| prev.and_then(|t| t.title.clone()));
    let status = update
        .status
        .or_else(|| prev.and_then(|t| t.status.clone()));
    let input_json = update
        .input_json
        .or_else(|| prev.and_then(|t| t.input_json.clone()));
    let output_text = match update.output_text {
        Some(next) => Some(merge_streaming_text(
            prev.and_then(|t| t.output_text.as_deref()),
            &next,
        )),
        None => prev.and_then(|t| t.output_text.clone()),
    };
    SessionTurnTool {
        session_id,
        tool_call_id: update.tool_call_id,
        turn_id,
        tool_kind,
        title,
        status,
        input_json,
        output_text,
        created_at,
        updated_at: now,
    }
}

fn tool_count_deltas(
    prev: Option<&SessionTurnTool>,
    next: &SessionTurnTool,
) -> (i64, i64, i64, i64, i64) {
    let prev_bucket = tool_status_bucket(prev.and_then(|t| t.status.as_deref()));
    let next_bucket = tool_status_bucket(next.status.as_deref());

    let mut delta_total = 0;
    let mut delta_pending = 0;
    let mut delta_running = 0;
    let mut delta_completed = 0;
    let mut delta_failed = 0;

    if prev.is_none() {
        delta_total += 1;
    }
    if prev_bucket != next_bucket {
        if let Some(bucket) = prev_bucket {
            match bucket {
                "pending" => delta_pending -= 1,
                "in_progress" => delta_running -= 1,
                "completed" => delta_completed -= 1,
                "failed" => delta_failed -= 1,
                _ => {}
            }
        }
        if let Some(bucket) = next_bucket {
            match bucket {
                "pending" => delta_pending += 1,
                "in_progress" => delta_running += 1,
                "completed" => delta_completed += 1,
                "failed" => delta_failed += 1,
                _ => {}
            }
        }
    }

    (
        delta_total,
        delta_pending,
        delta_running,
        delta_completed,
        delta_failed,
    )
}

fn tool_status_bucket(status: Option<&str>) -> Option<&'static str> {
    let s = status.unwrap_or("").to_lowercase();
    match s.as_str() {
        "pending" | "queued" => Some("pending"),
        "in_progress" | "inprogress" | "running" => Some("in_progress"),
        "completed" | "complete" | "ok" | "succeeded" => Some("completed"),
        "failed" | "error" => Some("failed"),
        "" => Some("pending"),
        _ => Some("pending"),
    }
}

fn normalize_tool_status(status: &str, event_type: SessionEventType) -> String {
    let s = status.trim().to_lowercase();
    if s == "inprogress" || s == "in_progress" || s == "running" {
        return "in_progress".to_string();
    }
    if s == "pending" || s == "queued" {
        return "pending".to_string();
    }
    if s == "completed" || s == "complete" || s == "ok" || s == "succeeded" {
        return "completed".to_string();
    }
    if s == "failed" || s == "error" {
        return "failed".to_string();
    }
    if matches!(event_type, SessionEventType::ToolResult) {
        return "completed".to_string();
    }
    if s.is_empty() {
        return "pending".to_string();
    }
    s
}

fn extract_tool_update(payload: &Value) -> &Value {
    payload.get("acp_update").unwrap_or(payload)
}

fn tool_call_id_from_payload(payload: &Value) -> Option<String> {
    if let Some(v) = payload.get("tool_call_id").and_then(|v| v.as_str()) {
        return Some(v.to_string());
    }
    let update = extract_tool_update(payload);
    let direct = update
        .get("toolCallId")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("tool_call_id").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        return Some(v.to_string());
    }
    let from_raw = update
        .pointer("/rawInput/call_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            update
                .pointer("/raw_input/call_id")
                .and_then(|v| v.as_str())
        });
    from_raw.map(|v| v.to_string())
}

fn extract_tool_output_text(update: &Value) -> Option<String> {
    let direct = update
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("output_text").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/toolCall/outputText")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            update
                .pointer("/toolCall/output_text")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.get("result").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/rawOutput/aggregated_output")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.pointer("/rawOutput/output").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let blocks = update.get("content").and_then(|v| v.as_array())?;
    let mut out = String::new();
    for b in blocks {
        if let Some(t) = b
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
        {
            out.push_str(t);
        } else if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out.trim().to_string())
    }
}

fn merge_streaming_text(prev: Option<&str>, next: &str) -> String {
    let prev = prev.unwrap_or("");
    if prev.is_empty() {
        return next.to_string();
    }
    if next.is_empty() {
        return prev.to_string();
    }
    if next.starts_with(prev) {
        return next.to_string();
    }
    if prev.starts_with(next) {
        return prev.to_string();
    }
    if next.len() >= prev.len() {
        next.to_string()
    } else {
        prev.to_string()
    }
}
