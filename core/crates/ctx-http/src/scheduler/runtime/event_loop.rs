use self::assistant::handle_assistant_complete;
use self::provider_events::{
    claim_init_provider_session_ref, enrich_done_payload, record_first_provider_event_metric,
};
use self::state::{
    should_check_store_terminal_status, should_drop_post_terminal_event,
    should_process_post_terminal_assistant_complete, EventLoopRuntimeState,
};
use self::terminal::{
    handle_done_event, handle_error_event, handle_session_gap_notice, handle_turn_interrupted,
    is_truthful_start_activity,
};
use self::tools::{handle_persisted_tool_event, prepare_tool_event_payload};
use super::helpers::should_track_thought_chunk;
use super::*;
use crate::scheduler::TurnStartProgress;
use ctx_core::ids::MessageId;
use ctx_session_tools::normalize_tool_event;
use ctx_session_tools::order_seq::attach_order_seq;
use std::sync::Weak;

mod assistant;
mod failure;
mod provider_events;
mod state;
mod telemetry;
mod terminal;
mod tools;

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
            record_first_provider_event_metric(&ctx, state.as_ref()).await;
        }

        if matches!(&ev.event_type, SessionEventType::Init) {
            claim_init_provider_session_ref(
                &mut ctx,
                state.as_ref(),
                &mut event_type,
                &mut payload,
            )
            .await;
        }

        if matches!(&ev.event_type, SessionEventType::Done) {
            enrich_done_payload(&ctx, &mut payload);
        }

        let allow_post_terminal_assistant_complete =
            should_process_post_terminal_assistant_complete(
                &event_type,
                runtime.terminal_status.as_ref(),
            );
        let dropped_by_store_terminal_status = should_check_store_terminal_status(&event_type)
            && should_drop_post_terminal_event(&ctx, &mut runtime).await
            && !should_process_post_terminal_assistant_complete(
                &event_type,
                runtime.terminal_status.as_ref(),
            );
        if (runtime.terminal_status.is_some() && !allow_post_terminal_assistant_complete)
            || dropped_by_store_terminal_status
        {
            tracing::debug!(
                session_id = %ctx.session_id.0,
                run_id = %ctx.run_id.0,
                turn_id = %ctx.turn_id.0,
                event_type = ?event_type,
                "dropping provider event after turn terminalization"
            );
            continue;
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

        if let Some(tool_event) = normalized_tool_event.as_ref() {
            payload =
                prepare_tool_event_payload(&ctx, state.as_ref(), &event_type, tool_event).await;
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
                    handle_persisted_tool_event(&ctx, &mut runtime, &event, tool_event).await;
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
