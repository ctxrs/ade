use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::Instant as TokioInstant;

use ctx_core::ids::MessageId;
use ctx_core::models::{Message, MessageDelivery, Session, SessionEventType};

use crate::daemon::AppState;
use crate::ops_events::OpsEvent;

mod interrupt_telemetry;
mod lifecycle;
mod persistence;
mod reconcile;
mod runtime;
mod terminal;

pub(crate) use interrupt_telemetry::{latency_bucket, metric_labels, InterruptTelemetryContext};
use lifecycle::{
    finalize_start_failure_if_needed, handle_provider_exit, handle_provider_stall,
    stop_running_turn, RunningTurn, StopReason,
};
use persistence::emit_event;
use runtime::start_turn;

pub use reconcile::{reconcile_turn_failed_on_provider_exit, reconcile_turn_terminal_state};
pub(crate) use runtime::model_context_window;

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
    Interrupt(InterruptTelemetryContext),
    StorageEmergency,
}

pub async fn session_worker(
    state: Arc<AppState>,
    session: Session,
    mut rx: mpsc::Receiver<SchedulerCommand>,
) {
    let mut session = session;
    let mut queue: VecDeque<QueuedMessage> = VecDeque::new();
    let store = match state.store_for_session(session.id).await {
        Ok(store) => store,
        Err(_) => return,
    };
    let order_seq_state = state.sessions.get_order_seq_state(&store, session.id).await;
    if let Ok(mut queued) = store.list_queued_messages_for_session(session.id).await {
        for m in queued.drain(..) {
            queue.push_back(QueuedMessage {
                message: m,
                enqueued_at: Instant::now(),
                run_id: None,
            });
        }
    }
    let mut running: Option<RunningTurn> = None;
    let mut running_inactivity_timeout: Option<Duration> = None;
    let mut running_inactivity_deadline: Option<TokioInstant> = None;
    let mut suspend_queue = false;
    let mut event_head_rx = state.subscribe_session_event_head(session.id).await;

    let worktree = match store.get_worktree(session.worktree_id).await {
        Ok(Some(wt)) => wt,
        _ => return,
    };
    let workdir = PathBuf::from(worktree.root_path.clone());
    let is_worktree = worktree.vcs_ref.is_some() || worktree.git_branch.is_some();
    let session_root_kind = if is_worktree {
        "worktree".to_string()
    } else {
        "workspace_root".to_string()
    };
    let mut worktree_event = OpsEvent::new("info", "worktree_resolved");
    worktree_event.session_id = Some(session.id.0.to_string());
    worktree_event.worktree_id = Some(session.worktree_id.0.to_string());
    worktree_event.worktree_root = Some(workdir.to_string_lossy().to_string());
    worktree_event.meta = Some(json!({
        "execution_environment": session.execution_environment.as_str(),
        "session_root_kind": session_root_kind.clone(),
        "vcs_kind": worktree.vcs_kind,
        "vcs_ref": worktree.vcs_ref,
        "git_branch": worktree.git_branch,
    }));
    state.telemetry.ops_events.emit(worktree_event);

    loop {
        if running.is_none() && !suspend_queue {
            if let Some(msg) = queue.pop_front() {
                let msg_id = msg.message.id;
                let msg_run_id = msg.message.run_id;
                let msg_turn_id = msg.message.turn_id;
                let session_for_turn = match store.get_session(session.id).await {
                    Ok(Some(fresh)) => {
                        session = fresh.clone();
                        fresh
                    }
                    _ => session.clone(),
                };
                if matches!(msg.message.delivery, MessageDelivery::Queued) {
                    let _ = emit_event(
                        &state,
                        session.id,
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
                // The runtime module owns provider/env/event-pump side effects so this loop stays
                // focused on queue progression and running-turn lifecycle.
                match start_turn(
                    &state,
                    &session_for_turn,
                    &workdir,
                    &session_root_kind,
                    msg,
                    Arc::clone(&order_seq_state),
                )
                .await
                {
                    Ok(turn) => {
                        state.set_running(session.id, true).await;
                        let timeout = state.provider_inactivity_timeout().await;
                        running_inactivity_timeout = Some(timeout);
                        running_inactivity_deadline = Some(TokioInstant::now() + timeout);
                        running = Some(turn);
                    }
                    Err(err) => {
                        let err_string = format!("{err:#}");
                        tracing::error!(
                            session_id = %session.id.0,
                            "failed to start turn: {err:#}"
                        );
                        if let Some(turn_id) = msg_turn_id {
                            finalize_start_failure_if_needed(
                                &state,
                                session.id,
                                msg_run_id,
                                turn_id,
                                msg_id,
                                &err_string,
                            )
                            .await;
                        }
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
                            let _ = stop_running_turn(
                                &state,
                                session.id,
                                turn,
                                StopReason::Cancel,
                                None,
                            )
                            .await;
                        }
                    }
                    Some(SchedulerCommand::Interrupt(interrupt)) => {
                        if let Some(turn) = running.take() {
                            suspend_queue = stop_running_turn(
                                &state,
                                session.id,
                                turn,
                                StopReason::Interrupt,
                                Some(interrupt),
                            )
                            .await;
                        }
                    }
                    Some(SchedulerCommand::StorageEmergency) => {
                        if let Some(turn) = running.take() {
                            suspend_queue = stop_running_turn(
                                &state,
                                session.id,
                                turn,
                                StopReason::StorageEmergency,
                                None,
                            )
                            .await;
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
                if let Some(turn) = running.take() {
                    handle_provider_exit(&state, session.id, turn).await;
                }
                state.set_running(session.id, false).await;
            }
            changed = event_head_rx.changed(), if running.is_some() => {
                if changed.is_err() {
                    event_head_rx = state.subscribe_session_event_head(session.id).await;
                }
                if let Some(timeout) = running_inactivity_timeout {
                    running_inactivity_deadline = Some(TokioInstant::now() + timeout);
                }
            }
            _ = async {
                if let Some(deadline) = running_inactivity_deadline {
                    tokio::time::sleep_until(deadline).await;
                }
            }, if running.is_some() && running_inactivity_deadline.is_some() => {
                if let Some(turn) = running.take() {
                    handle_provider_stall(&state, session.id, turn).await;
                }
                state.set_running(session.id, false).await;
            }
        }
    }
}
