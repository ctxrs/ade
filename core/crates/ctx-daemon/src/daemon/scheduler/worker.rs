use std::sync::Weak;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant as TokioInstant;

use ctx_core::models::Session;

use crate::daemon::DaemonState;

use super::lifecycle::{handle_provider_exit, RunningTurn};
use super::SchedulerCommand;

mod bootstrap;
mod commands;
mod deadlines;
mod queue;

use self::bootstrap::{bootstrap_worker, WorkerBootstrap};
use self::commands::{handle_scheduler_command, SchedulerCommandAction};
use self::deadlines::{
    handle_inactivity_deadline_elapsed, handle_start_deadline_elapsed, refresh_inactivity_deadline,
    WorkerDeadlineAction,
};
use self::queue::{start_next_queued_turn, QueueStartContext, QueueStartOutcome};

pub(super) async fn session_worker(
    state_weak: Weak<DaemonState>,
    session: Session,
    mut rx: mpsc::Receiver<SchedulerCommand>,
) {
    // Keep only a weak reference in the scheduler task so background workers do not
    // keep DaemonState alive after tests or shutdown drop the owner.
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
            match start_next_queued_turn(QueueStartContext {
                state_weak: &state_weak,
                session: &mut session,
                store: &store,
                queue: &mut queue,
                workdir: &workdir,
                session_root_kind: &session_root_kind,
                order_seq_state: &order_seq_state,
                running: &mut running,
                running_inactivity_timeout: &mut running_inactivity_timeout,
                running_inactivity_deadline: &mut running_inactivity_deadline,
                running_start_deadline: &mut running_start_deadline,
            })
            .await
            {
                QueueStartOutcome::Idle => {}
                QueueStartOutcome::StartedOrFailed => continue,
                QueueStartOutcome::StopWorker => break,
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
                    event_head_rx = state.subscribe_session_event_head(session.id).await;
                }
                refresh_inactivity_deadline(
                    running_inactivity_timeout,
                    &mut running_inactivity_deadline,
                );
            }
            _ = async {
                if let Some(deadline) = running_start_deadline {
                    tokio::time::sleep_until(deadline).await;
                }
            }, if running.is_some() && running_start_deadline.is_some() => {
                if matches!(
                    handle_start_deadline_elapsed(
                        &state_weak,
                        session.id,
                        &mut running,
                        &mut running_start_deadline,
                    )
                    .await,
                    WorkerDeadlineAction::Break
                ) {
                    break;
                }
            }
            _ = async {
                if let Some(deadline) = running_inactivity_deadline {
                    tokio::time::sleep_until(deadline).await;
                }
            }, if running.is_some() && running_inactivity_deadline.is_some() => {
                if matches!(
                    handle_inactivity_deadline_elapsed(
                        &state_weak,
                        session.id,
                        &mut running,
                        &mut running_start_deadline,
                        &mut suspend_queue,
                    )
                    .await,
                    WorkerDeadlineAction::Break
                ) {
                    break;
                }
            }
        }
    }
}
