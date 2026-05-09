use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::Instant as TokioInstant;

use ctx_core::models::{MessageDelivery, Session, SessionEventType};

use crate::daemon::AppState;

use super::lifecycle::{
    fail_starting_turn, finalize_start_failure_if_needed, handle_provider_exit,
    handle_provider_stall, RunningTurn, TurnStartProgress,
};
use super::persistence::emit_event;
use super::runtime::start_turn;
use super::SchedulerCommand;

mod bootstrap;
mod commands;

use self::bootstrap::{bootstrap_worker, WorkerBootstrap};
use self::commands::{handle_scheduler_command, SchedulerCommandAction};

pub(super) async fn session_worker(
    state_weak: Weak<AppState>,
    session: Session,
    mut rx: mpsc::Receiver<SchedulerCommand>,
) {
    // Keep only a weak reference in the scheduler task so background workers do not
    // keep AppState alive after tests or shutdown drop the owner.
    let Some(state) = state_weak.upgrade() else {
        return;
    };
    let mut session = session;
    let Some(bootstrap) = bootstrap_worker(&state, &session).await else {
        return;
    };
    let WorkerBootstrap {
        store,
        order_seq_state,
        mut queue,
        mut event_head_rx,
        workdir,
        session_root_kind,
    } = bootstrap;
    let mut running: Option<RunningTurn> = None;
    let mut running_inactivity_timeout: Option<Duration> = None;
    let mut running_inactivity_deadline: Option<TokioInstant> = None;
    let mut running_start_deadline: Option<TokioInstant> = None;
    let mut suspend_queue = false;
    drop(state);

    loop {
        if running.is_none() && !suspend_queue {
            if let Some(msg) = queue.pop_front() {
                let Some(state) = state_weak.upgrade() else {
                    break;
                };
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
                        let timeout = state.sessions.provider_inactivity_timeout().await;
                        running_inactivity_timeout = Some(timeout);
                        running_inactivity_deadline = Some(TokioInstant::now() + timeout);
                        running_start_deadline = Some(turn.start_deadline);
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
                        running_start_deadline = None;
                    }
                }
                continue;
            }
        }

        tokio::select! {
            cmd = rx.recv() => {
                if matches!(
                    handle_scheduler_command(
                        cmd,
                        &state_weak,
                        session.id,
                        &mut queue,
                        &mut running,
                        &mut running_start_deadline,
                        &mut suspend_queue,
                    )
                    .await,
                    SchedulerCommandAction::Break
                ) {
                    break;
                }
            }
            _ = async {
                if let Some(turn) = running.as_mut() {
                    let _ = (&mut turn.handle.done).await;
                }
            }, if running.is_some() => {
                if let Some(turn) = running.take() {
                    running_start_deadline = None;
                    let Some(state) = state_weak.upgrade() else {
                        break;
                    };
                    let finalized = handle_provider_exit(&state, session.id, turn).await;
                    suspend_queue = !finalized;
                    state.set_running(session.id, false).await;
                } else if let Some(state) = state_weak.upgrade() {
                    state.set_running(session.id, false).await;
                } else {
                    break;
                }
            }
            changed = event_head_rx.changed(), if running.is_some() => {
                let Some(state) = state_weak.upgrade() else {
                    break;
                };
                if changed.is_err() {
                    event_head_rx = state.sessions.subscribe_session_event_head(session.id).await;
                }
                if let Some(timeout) = running_inactivity_timeout {
                    running_inactivity_deadline = Some(TokioInstant::now() + timeout);
                }
            }
            _ = async {
                if let Some(deadline) = running_start_deadline {
                    tokio::time::sleep_until(deadline).await;
                }
            }, if running.is_some() && running_start_deadline.is_some() => {
                let start_still_pending = running
                    .as_ref()
                    .is_some_and(|turn| *turn.start_progress.borrow() == TurnStartProgress::Pending);
                running_start_deadline = None;
                if start_still_pending {
                    if let Some(turn) = running.take() {
                        let Some(state) = state_weak.upgrade() else {
                            break;
                        };
                        fail_starting_turn(
                            &state,
                            session.id,
                            turn,
                            "provider did not report turn start before deadline",
                        )
                        .await;
                        state.set_running(session.id, false).await;
                    } else if let Some(state) = state_weak.upgrade() {
                        state.set_running(session.id, false).await;
                    } else {
                        break;
                    }
                }
            }
            _ = async {
                if let Some(deadline) = running_inactivity_deadline {
                    tokio::time::sleep_until(deadline).await;
                }
            }, if running.is_some() && running_inactivity_deadline.is_some() => {
                if let Some(turn) = running.take() {
                    running_start_deadline = None;
                    let Some(state) = state_weak.upgrade() else {
                        break;
                    };
                    let finalized = handle_provider_stall(&state, session.id, turn).await;
                    suspend_queue = !finalized;
                    state.set_running(session.id, false).await;
                } else if let Some(state) = state_weak.upgrade() {
                    state.set_running(session.id, false).await;
                } else {
                    break;
                }
            }
        }
    }
}
