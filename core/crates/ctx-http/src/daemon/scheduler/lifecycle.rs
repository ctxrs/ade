use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant as TokioInstant;

use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{SessionEventType, SessionTurnStatus};
use ctx_providers::adapters::{ProviderAdapter, ProviderTurnOutcome, RunHandle};
use ctx_providers::events::NormalizedEvent;

use crate::daemon::AppState;

use super::reconcile::reconcile_turn_terminal_state;
use super::terminal::{finalize_failed_turn, finalize_provider_outcome, FailedTurnTerminalization};

mod stop;

pub(crate) use stop::stop_running_turn;

pub(crate) struct RunningTurn {
    pub(crate) adapter: Arc<dyn ProviderAdapter>,
    pub(crate) handle: RunHandle,
    pub(crate) run_id: RunId,
    pub(crate) turn_id: TurnId,
    pub(crate) message_id: MessageId,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) execution_environment_label: String,
    pub(crate) session_root_kind: String,
    pub(crate) event_tx: mpsc::Sender<NormalizedEvent>,
    pub(crate) events_done: Option<oneshot::Receiver<()>>,
    pub(crate) start_progress: watch::Receiver<TurnStartProgress>,
    pub(crate) start_deadline: TokioInstant,
    pub(crate) mcp_token: Option<String>,
}

const PROVIDER_OUTCOME_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const TURN_EVENT_LOOP_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const TURN_TERMINALIZATION_RETRY_LIMIT: usize = 3;
const TURN_TERMINALIZATION_RETRY_BASE_MS: u64 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnStartProgress {
    Pending,
    Started,
    Terminal,
}

#[derive(Clone, Copy)]
pub(crate) enum StopReason {
    Cancel,
    Interrupt,
    StorageEmergency,
}

async fn wait_for_turn_event_loop(
    session_id: SessionId,
    run_id: RunId,
    turn_id: TurnId,
    events_done: oneshot::Receiver<()>,
) {
    if tokio::time::timeout(TURN_EVENT_LOOP_DRAIN_TIMEOUT, events_done)
        .await
        .is_err()
    {
        tracing::warn!(
            session_id = %session_id.0,
            run_id = %run_id.0,
            turn_id = %turn_id.0,
            drain_timeout_ms = TURN_EVENT_LOOP_DRAIN_TIMEOUT.as_millis(),
            "turn event loop did not drain before terminal fallback; finalizing from scheduler outcome"
        );
    }
}

fn abort_provider(handle: &mut RunHandle) {
    if let Some(abort) = handle.abort.take() {
        abort.abort();
    }
}

fn provider_protocol_violation(reason: &str, message: &str) -> ProviderTurnOutcome {
    ProviderTurnOutcome::protocol_violation(reason, message)
}

async fn revoke_turn_mcp_token(state: &Arc<AppState>, token: &mut Option<String>) {
    if let Some(token) = token.take() {
        crate::daemon::revoke_provider_session_mcp_token(state.as_ref(), &token).await;
    }
}

async fn wait_for_provider_outcome(
    handle: &mut RunHandle,
    closed_fallback: ProviderTurnOutcome,
    timeout_fallback: ProviderTurnOutcome,
) -> ProviderTurnOutcome {
    match tokio::time::timeout(PROVIDER_OUTCOME_WAIT_TIMEOUT, &mut handle.outcome).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) => {
            abort_provider(handle);
            closed_fallback
        }
        Err(_) => {
            abort_provider(handle);
            timeout_fallback
        }
    }
}

async fn turn_finished_persisted(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn_id: TurnId,
) -> bool {
    let Ok(store) = state.store_for_session(session_id).await else {
        return false;
    };
    match store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await
    {
        Ok(events) => events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::TurnFinished)),
        Err(err) => {
            tracing::warn!(
                session_id = %session_id.0,
                turn_id = %turn_id.0,
                "failed to verify durable TurnFinished after terminalization: {err:#}"
            );
            false
        }
    }
}

async fn finalize_provider_outcome_required(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    message_id: MessageId,
    outcome: ProviderTurnOutcome,
) -> bool {
    for attempt in 0..=TURN_TERMINALIZATION_RETRY_LIMIT {
        match finalize_provider_outcome(
            state,
            session_id,
            run_id,
            turn_id,
            message_id,
            outcome.clone(),
        )
        .await
        {
            Ok(()) if turn_finished_persisted(state, session_id, turn_id).await => return true,
            Ok(()) => {
                tracing::warn!(
                    session_id = %session_id.0,
                    turn_id = %turn_id.0,
                    attempt,
                    "turn terminalization completed without durable TurnFinished"
                );
            }
            Err(err) => {
                tracing::warn!(
                    session_id = %session_id.0,
                    turn_id = %turn_id.0,
                    attempt,
                    "turn terminalization failed: {err:#}"
                );
            }
        }

        if attempt < TURN_TERMINALIZATION_RETRY_LIMIT {
            let backoff_ms =
                TURN_TERMINALIZATION_RETRY_BASE_MS.saturating_mul((attempt + 1) as u64);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        }
    }

    tracing::error!(
        session_id = %session_id.0,
        turn_id = %turn_id.0,
        "turn terminalization exhausted retries without durable TurnFinished"
    );
    false
}

pub(crate) async fn handle_provider_exit(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
) -> bool {
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    let message_id = turn.message_id;
    let outcome = wait_for_provider_outcome(
        &mut turn.handle,
        provider_protocol_violation(
            "provider_protocol_violation_missing_outcome",
            "provider exited without reporting a terminal outcome",
        ),
        provider_protocol_violation(
            "provider_protocol_violation_outcome_timeout",
            "provider exited without reporting a terminal outcome before timeout",
        ),
    )
    .await;
    drop(turn.event_tx);
    let finalized = if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
        finalize_provider_outcome_required(
            state,
            session_id,
            Some(run_id),
            turn_id,
            message_id,
            outcome,
        )
        .await
    } else {
        finalize_provider_outcome_required(
            state,
            session_id,
            Some(run_id),
            turn_id,
            message_id,
            outcome,
        )
        .await
    };
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
    finalized
}

pub(crate) async fn handle_provider_stall(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
) -> bool {
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    let message_id = turn.message_id;
    let _ = turn.adapter.cancel(&mut turn.handle).await;
    let _ = wait_for_provider_outcome(
        &mut turn.handle,
        provider_protocol_violation(
            "provider_protocol_violation_inactivity_timeout",
            "provider stalled without reporting a terminal outcome",
        ),
        provider_protocol_violation(
            "provider_protocol_violation_inactivity_timeout",
            "provider stalled without reporting a terminal outcome before timeout",
        ),
    )
    .await;
    drop(turn.event_tx);
    if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
    }
    let outcome = provider_protocol_violation(
        "provider_protocol_violation_inactivity_timeout",
        "provider stalled without reporting a terminal outcome before timeout",
    );
    let finalized = finalize_provider_outcome_required(
        state,
        session_id,
        Some(run_id),
        turn_id,
        message_id,
        outcome,
    )
    .await;
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
    finalized
}

pub(crate) async fn fail_starting_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
    error_message: &str,
) {
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    let message_id = turn.message_id;
    let _ = turn.adapter.cancel(&mut turn.handle).await;
    let _ = wait_for_provider_outcome(
        &mut turn.handle,
        provider_protocol_violation("start_not_acknowledged", error_message),
        provider_protocol_violation("start_not_acknowledged", error_message),
    )
    .await;
    drop(turn.event_tx);
    if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
    }
    let _ = finalize_failed_turn(
        state,
        session_id,
        Some(run_id),
        turn_id,
        message_id,
        FailedTurnTerminalization {
            message: error_message,
            reason: Some("start_not_acknowledged"),
            details: None,
            kind: Some(json!("start_not_acknowledged")),
            emit_error_event: true,
        },
    )
    .await;
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
}

fn has_terminal_event(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::Done
            | SessionEventType::Error
            | SessionEventType::TurnInterrupted
            | SessionEventType::TurnFinished
    )
}

pub(crate) async fn finalize_start_failure_if_needed(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    message_id: MessageId,
    error_message: &str,
) {
    let Ok(store) = state.store_for_session(session_id).await else {
        return;
    };
    let turn = store
        .get_session_turn(session_id, turn_id)
        .await
        .ok()
        .flatten();
    if turn.as_ref().is_some_and(|turn| {
        matches!(
            turn.status,
            SessionTurnStatus::Completed
                | SessionTurnStatus::Failed
                | SessionTurnStatus::Interrupted
        )
    }) {
        return;
    }

    if let Ok(events) = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await
    {
        if events
            .iter()
            .any(|event| has_terminal_event(&event.event_type))
        {
            let _ =
                reconcile_turn_terminal_state(state, session_id, run_id, turn_id, "start_failed")
                    .await;
            return;
        }
    }

    let _ = finalize_failed_turn(
        state,
        session_id,
        run_id,
        turn_id,
        message_id,
        FailedTurnTerminalization {
            message: error_message,
            reason: Some("start_failed"),
            details: None,
            kind: Some(json!("start_failed")),
            emit_error_event: true,
        },
    )
    .await;
}
