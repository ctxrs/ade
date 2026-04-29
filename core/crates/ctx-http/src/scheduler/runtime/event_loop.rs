use self::assistant::handle_assistant_complete;
use self::terminal::{
    handle_done_event, handle_error_event, handle_session_gap_notice, handle_turn_interrupted,
    is_truthful_start_activity,
};
use super::helpers::{read_codex_context_window_metrics, should_track_thought_chunk};
use super::*;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::scheduler::TurnStartProgress;
use ctx_core::ids::MessageId;
use std::collections::HashMap;
use std::sync::Weak;

mod assistant;
mod failure;
mod terminal;

pub(super) struct TurnEventLoop {
    pub(super) state_weak: Weak<AppState>,
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

impl TurnEventLoop {
    fn state(&self) -> Option<Arc<AppState>> {
        self.state_weak.upgrade()
    }
}

struct EventLoopRuntimeState {
    assistant_partial: String,
    assistant_partial_message_id: Option<String>,
    assistant_sequence: i64,
    assistant_emitted: String,
    thought_partial: String,
    tool_cache: HashMap<String, SessionTurnTool>,
    terminal_status: Option<SessionTurnStatus>,
    start_progress: TurnStartProgress,
    first_event_seen: bool,
    telemetry_emitted: bool,
}

impl Default for EventLoopRuntimeState {
    fn default() -> Self {
        Self {
            assistant_partial: String::new(),
            assistant_partial_message_id: None,
            assistant_sequence: 0,
            assistant_emitted: String::new(),
            thought_partial: String::new(),
            tool_cache: HashMap::new(),
            terminal_status: None,
            start_progress: TurnStartProgress::Pending,
            first_event_seen: false,
            telemetry_emitted: false,
        }
    }
}

impl EventLoopRuntimeState {
    fn mark_first_event_seen(&mut self) -> bool {
        if self.first_event_seen {
            return false;
        }
        self.first_event_seen = true;
        true
    }

    fn promote_started_if_pending(
        &mut self,
        start_progress_tx: &tokio::sync::watch::Sender<TurnStartProgress>,
    ) -> bool {
        if self.start_progress != TurnStartProgress::Pending {
            return false;
        }
        self.start_progress = TurnStartProgress::Started;
        let _ = start_progress_tx.send(TurnStartProgress::Started);
        true
    }

    fn promote_terminal(
        &mut self,
        start_progress_tx: &tokio::sync::watch::Sender<TurnStartProgress>,
    ) {
        if self.start_progress == TurnStartProgress::Terminal {
            return;
        }
        self.start_progress = TurnStartProgress::Terminal;
        let _ = start_progress_tx.send(TurnStartProgress::Terminal);
    }
}

pub(super) fn spawn_turn_event_loop(ctx: TurnEventLoop) {
    tokio::spawn(async move {
        run_turn_event_loop(ctx).await;
    });
}

async fn run_turn_event_loop(mut ctx: TurnEventLoop) {
    let mut runtime = EventLoopRuntimeState::default();

    while let Some(ev) = ctx.ev_rx.recv().await {
        let Some(state) = ctx.state() else {
            break;
        };
        let mut event_type = ev.event_type.clone();
        let raw_payload = ev.payload_json.clone();
        let mut payload = raw_payload.clone();

        if runtime.mark_first_event_seen() {
            let first_ms = ctx.run_started_at.elapsed().as_millis() as u64;
            let mut first_labels = HashMap::new();
            first_labels.insert("provider_id".to_string(), ctx.provider_id.clone());
            first_labels.insert("model_id".to_string(), ctx.model_id.clone());
            first_labels.insert(
                "execution_environment".to_string(),
                ctx.execution_environment_label.clone(),
            );
            first_labels.insert(
                "session_root_kind".to_string(),
                ctx.session_root_kind.clone(),
            );
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
                .record_metric(first_metric, ctx.perf_run_id.clone(), None, None)
                .await;
        }

        if matches!(&ev.event_type, SessionEventType::Init) {
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
                match ctx
                    .store
                    .claim_session_provider_session_ref(
                        ctx.session_id,
                        ps.to_string(),
                        "scheduler.init_event",
                    )
                    .await
                {
                    Ok(()) => {
                        ctx.provider_session_ref = Some(ps.to_string());
                    }
                    Err(err) => {
                        event_type = SessionEventType::Error;
                        payload = json!({
                            "message": err.to_string(),
                            "reason": "provider_session_ref_claim_failed",
                            "kind": "provider_session_ref_claim_failed",
                            "details": {
                                "provider_session_id": ps,
                                "provider_id": ctx.provider_id.clone(),
                            },
                        });
                    }
                }
            }
        }

        if matches!(&ev.event_type, SessionEventType::Done) {
            if let Some(obj) = payload.as_object_mut() {
                if obj.get("context_window").is_none() {
                    let metrics = if ctx.provider_id == "codex" {
                        ctx.codex_home
                            .as_deref()
                            .and_then(|home| {
                                ctx.provider_session_ref.as_deref().and_then(|session_ref| {
                                    read_codex_context_window_metrics(home, session_ref)
                                })
                            })
                            .or_else(|| ctx.context_window_metrics.clone())
                    } else {
                        ctx.context_window_metrics.clone()
                    };
                    if let Some(metrics) = metrics {
                        obj.entry("context_window").or_insert(metrics);
                    }
                }
                obj.entry("status").or_insert(json!("completed"));
            }
        }

        let normalized_tool_event = if matches!(
            &event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        ) {
            Some(normalize_tool_event(&event_type, &raw_payload))
        } else {
            None
        };

        if matches!(&event_type, SessionEventType::ToolCall) {
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
                event.session_id = Some(ctx.session_id.0.to_string());
                event.worktree_id = Some(ctx.worktree_id.0.to_string());
                event.run_id = Some(ctx.run_id.0.to_string());
                event.turn_id = Some(ctx.turn_id.0.to_string());
                event.provider_id = Some(ctx.provider_id.clone());
                event.tool_kind = tool_meta.tool_kind.clone();
                event.cwd = tool_meta.cwd.clone();
                event.worktree_root = Some(ctx.workdir_str.clone());
                event.meta = if meta.is_empty() {
                    None
                } else {
                    Some(Value::Object(meta))
                };
                state.telemetry.ops_events.emit(event);

                if let Some(cwd) = tool_meta.cwd.as_deref() {
                    if cwd_outside_worktree(cwd, &ctx.workdir_root, ctx.workdir_canonical.as_ref())
                    {
                        let mut warn_event = OpsEvent::new("warn", "tool_exec_anomaly");
                        warn_event.session_id = Some(ctx.session_id.0.to_string());
                        warn_event.worktree_id = Some(ctx.worktree_id.0.to_string());
                        warn_event.run_id = Some(ctx.run_id.0.to_string());
                        warn_event.turn_id = Some(ctx.turn_id.0.to_string());
                        warn_event.provider_id = Some(ctx.provider_id.clone());
                        warn_event.tool_kind = tool_meta.tool_kind.clone();
                        warn_event.cwd = Some(cwd.to_string());
                        warn_event.worktree_root = Some(ctx.workdir_str.clone());
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
            let output_artifact = if matches!(&event_type, SessionEventType::ToolResult) {
                maybe_spool_tool_output(
                    state.as_ref(),
                    &ctx.store,
                    tool_event,
                    tool_runtime::ToolOutputArtifactScope {
                        session_id: ctx.session_id,
                        task_id: ctx.task_id,
                        workspace_id: ctx.workspace_id,
                        worktree_id: ctx.worktree_id,
                        turn_id: ctx.turn_id,
                    },
                )
                .await
            } else {
                None
            };
            payload = sanitize_normalized_tool_event_payload(tool_event, output_artifact.as_ref());
        }

        {
            let mut order_seq_state = ctx.order_seq_state.lock().await;
            attach_order_seq(
                &mut order_seq_state,
                &event_type,
                &mut payload,
                Some(&ctx.turn_id),
                runtime.assistant_sequence,
            );
        }

        let event = match append_session_event_with_retry(
            &ctx.store,
            ctx.session_id,
            Some(ctx.run_id),
            Some(ctx.turn_id),
            event_type.clone(),
            payload,
        )
        .await
        {
            Ok(event) => event,
            Err(err) => {
                tracing::warn!(
                    session_id = %ctx.session_id.0,
                    run_id = %ctx.run_id.0,
                    turn_id = %ctx.turn_id.0,
                    event_type = ?event_type,
                    "failed to append session event: {err:#}"
                );
                continue;
            }
        };

        let publish_after_persist = matches!(
            &event.event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        );
        if !publish_after_persist {
            state.publish_event(event.clone()).await;
        }

        if is_truthful_start_activity(&event.event_type)
            && runtime.promote_started_if_pending(&ctx.start_progress_tx)
        {
            let _ = ctx
                .store
                .update_session_turn_status(
                    ctx.session_id,
                    ctx.turn_id,
                    SessionTurnStatus::Running,
                    None,
                    None,
                    event.created_at,
                )
                .await;
        }

        match &event.event_type {
            SessionEventType::AssistantChunk => {
                if let Some(fragment) = raw_payload.get("content_fragment").and_then(Value::as_str)
                {
                    runtime.assistant_partial.push_str(fragment);
                    if let Some(message_id) = raw_payload
                        .get("message_id")
                        .and_then(Value::as_str)
                        .map(|value| value.to_string())
                    {
                        runtime.assistant_partial_message_id = Some(message_id);
                    }
                }
            }
            SessionEventType::ThoughtChunk if should_track_thought_chunk(&raw_payload) => {
                if let Some(fragment) = raw_payload.get("content_fragment").and_then(Value::as_str)
                {
                    runtime.thought_partial.push_str(fragment);
                    let _ = ctx
                        .store
                        .update_session_turn_partial(
                            ctx.session_id,
                            ctx.turn_id,
                            None,
                            Some(&runtime.thought_partial),
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
                handle_session_gap_notice(&ctx, &event).await;
            }
            SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult => {
                if let Some(tool_event) = normalized_tool_event.as_ref() {
                    let order_seq = read_order_seq(&event.payload_json);
                    if let Some(update) = build_turn_tool_update(tool_event, order_seq) {
                        let prev = if matches!(&event.event_type, SessionEventType::ToolCallUpdate)
                        {
                            runtime.tool_cache.get(&update.tool_call_id).cloned()
                        } else if let Some(cached) =
                            runtime.tool_cache.get(&update.tool_call_id).cloned()
                        {
                            Some(cached)
                        } else {
                            ctx.store
                                .get_session_turn_tool(ctx.session_id, &update.tool_call_id)
                                .await
                                .ok()
                                .flatten()
                        };
                        if let Some(merged) = merge_tool_update(
                            prev.as_ref(),
                            update,
                            ctx.session_id,
                            ctx.turn_id,
                            event.seq,
                            event.created_at,
                        ) {
                            if matches!(&event.event_type, SessionEventType::ToolCallUpdate) {
                                runtime
                                    .tool_cache
                                    .insert(merged.tool_call_id.clone(), merged);
                            } else {
                                let (
                                    delta_total,
                                    delta_pending,
                                    delta_running,
                                    delta_completed,
                                    delta_failed,
                                ) = tool_count_deltas(prev.as_ref(), &merged);
                                let _ = ctx.store.upsert_session_turn_tool(merged.clone()).await;
                                if delta_total != 0
                                    || delta_pending != 0
                                    || delta_running != 0
                                    || delta_completed != 0
                                    || delta_failed != 0
                                {
                                    let _ = ctx
                                        .store
                                        .update_session_turn_tool_counts(
                                            ctx.session_id,
                                            ctx.turn_id,
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
                                runtime
                                    .tool_cache
                                    .insert(merged.tool_call_id.clone(), merged);
                            }
                        }
                    }
                }
            }
            SessionEventType::AssistantComplete => {
                handle_assistant_complete(&ctx, &mut runtime, &event).await;
            }
            SessionEventType::Done => {
                handle_done_event(&ctx, &mut runtime).await;
            }
            SessionEventType::TurnInterrupted => {
                handle_turn_interrupted(&ctx, &mut runtime, &event).await;
            }
            SessionEventType::Error => {
                handle_error_event(&ctx, &mut runtime, &event).await;
            }
            _ => {}
        }

        if publish_after_persist {
            state.publish_event(event.clone()).await;
        }
    }
    let _ = ctx.events_done_tx.send(());
}

#[cfg(test)]
mod tests;
