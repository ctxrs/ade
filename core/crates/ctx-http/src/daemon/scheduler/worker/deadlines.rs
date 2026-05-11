use std::sync::Weak;
use std::time::Duration;

use tokio::time::Instant as TokioInstant;

use ctx_core::ids::SessionId;

use crate::daemon::scheduler::lifecycle::{
    fail_starting_turn, handle_provider_stall, RunningTurn, TurnStartProgress,
};
use crate::daemon::AppState;

pub(super) enum WorkerDeadlineAction {
    Continue,
    Break,
}

pub(super) fn refresh_inactivity_deadline(
    timeout: Option<Duration>,
    running_inactivity_deadline: &mut Option<TokioInstant>,
) {
    if let Some(timeout) = timeout {
        *running_inactivity_deadline = Some(TokioInstant::now() + timeout);
    }
}

pub(super) async fn handle_start_deadline_elapsed(
    state_weak: &Weak<AppState>,
    session_id: SessionId,
    running: &mut Option<RunningTurn>,
    running_start_deadline: &mut Option<TokioInstant>,
) -> WorkerDeadlineAction {
    let start_still_pending = running
        .as_ref()
        .is_some_and(|turn| *turn.start_progress.borrow() == TurnStartProgress::Pending);
    *running_start_deadline = None;

    if !start_still_pending {
        return WorkerDeadlineAction::Continue;
    }

    let Some(turn) = running.take() else {
        return clear_running_or_break(state_weak, session_id).await;
    };
    let Some(state) = state_weak.upgrade() else {
        return WorkerDeadlineAction::Break;
    };
    fail_starting_turn(
        &state,
        session_id,
        turn,
        "provider did not report turn start before deadline",
    )
    .await;
    state.set_running(session_id, false).await;
    WorkerDeadlineAction::Continue
}

pub(super) async fn handle_inactivity_deadline_elapsed(
    state_weak: &Weak<AppState>,
    session_id: SessionId,
    running: &mut Option<RunningTurn>,
    running_start_deadline: &mut Option<TokioInstant>,
    suspend_queue: &mut bool,
) -> WorkerDeadlineAction {
    let Some(turn) = running.take() else {
        return clear_running_or_break(state_weak, session_id).await;
    };
    *running_start_deadline = None;
    let Some(state) = state_weak.upgrade() else {
        return WorkerDeadlineAction::Break;
    };
    let finalized = handle_provider_stall(&state, session_id, turn).await;
    *suspend_queue = !finalized;
    state.set_running(session_id, false).await;
    WorkerDeadlineAction::Continue
}

async fn clear_running_or_break(
    state_weak: &Weak<AppState>,
    session_id: SessionId,
) -> WorkerDeadlineAction {
    let Some(state) = state_weak.upgrade() else {
        return WorkerDeadlineAction::Break;
    };
    state.set_running(session_id, false).await;
    WorkerDeadlineAction::Continue
}
