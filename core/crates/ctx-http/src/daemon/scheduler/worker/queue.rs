use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::Instant as TokioInstant;

use ctx_core::models::{MessageDelivery, Session, SessionEventType};
use ctx_session_tools::order_seq::OrderSeqState;

use crate::daemon::scheduler::lifecycle::{finalize_start_failure_if_needed, RunningTurn};
use crate::daemon::scheduler::persistence::emit_event;
use crate::daemon::scheduler::runtime::start_turn;
use crate::daemon::scheduler::QueuedMessage;
use crate::daemon::AppState;

pub(super) enum QueueStartOutcome {
    Idle,
    StartedOrFailed,
    StopWorker,
}

pub(super) struct QueueStartContext<'a> {
    pub(super) state_weak: &'a Weak<AppState>,
    pub(super) session: &'a mut Session,
    pub(super) store: &'a ctx_store::Store,
    pub(super) queue: &'a mut VecDeque<QueuedMessage>,
    pub(super) workdir: &'a Path,
    pub(super) session_root_kind: &'a str,
    pub(super) order_seq_state: &'a Arc<Mutex<OrderSeqState>>,
    pub(super) running: &'a mut Option<RunningTurn>,
    pub(super) running_inactivity_timeout: &'a mut Option<Duration>,
    pub(super) running_inactivity_deadline: &'a mut Option<TokioInstant>,
    pub(super) running_start_deadline: &'a mut Option<TokioInstant>,
}

pub(super) async fn start_next_queued_turn(ctx: QueueStartContext<'_>) -> QueueStartOutcome {
    let Some(msg) = ctx.queue.pop_front() else {
        return QueueStartOutcome::Idle;
    };
    let Some(state) = ctx.state_weak.upgrade() else {
        return QueueStartOutcome::StopWorker;
    };
    let msg_id = msg.message.id;
    let msg_run_id = msg.message.run_id;
    let msg_turn_id = msg.message.turn_id;
    let session_for_turn = match ctx.store.get_session(ctx.session.id).await {
        Ok(Some(fresh)) => {
            *ctx.session = fresh.clone();
            fresh
        }
        _ => ctx.session.clone(),
    };
    if matches!(msg.message.delivery, MessageDelivery::Queued) {
        let _ = emit_event(
            &state,
            ctx.session.id,
            msg.message.run_id,
            msg.message.turn_id,
            SessionEventType::MessageQueuePromoted,
            json!({
                "message_id": msg.message.id.0,
                "previous_position": 0,
            }),
        )
        .await;
    }
    // The runtime module owns provider/env/event-pump side effects; this helper only bridges
    // queue progression to the running-turn lifecycle tracked by the worker loop.
    match start_turn(
        &state,
        &session_for_turn,
        ctx.workdir,
        ctx.session_root_kind,
        msg,
        Arc::clone(ctx.order_seq_state),
    )
    .await
    {
        Ok(turn) => {
            state.set_running(ctx.session.id, true).await;
            let timeout = state.sessions.provider_inactivity_timeout().await;
            *ctx.running_inactivity_timeout = Some(timeout);
            *ctx.running_inactivity_deadline = Some(TokioInstant::now() + timeout);
            *ctx.running_start_deadline = Some(turn.start_deadline);
            *ctx.running = Some(turn);
        }
        Err(err) => {
            let err_string = format!("{err:#}");
            tracing::error!(
                session_id = %ctx.session.id.0,
                "failed to start turn: {err:#}"
            );
            if let Some(turn_id) = msg_turn_id {
                finalize_start_failure_if_needed(
                    &state,
                    ctx.session.id,
                    msg_run_id,
                    turn_id,
                    msg_id,
                    &err_string,
                )
                .await;
            }
            state.set_running(ctx.session.id, false).await;
            *ctx.running = None;
            *ctx.running_start_deadline = None;
        }
    }
    QueueStartOutcome::StartedOrFailed
}
