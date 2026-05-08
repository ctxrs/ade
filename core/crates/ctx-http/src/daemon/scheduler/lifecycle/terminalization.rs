use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::oneshot;

use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{SessionEventType, SessionTurnStatus};
use ctx_providers::adapters::{ProviderTurnOutcome, RunHandle};

use crate::daemon::AppState;

use super::super::reconcile::reconcile_turn_terminal_state;
use super::super::terminal::{
    finalize_failed_turn, finalize_provider_outcome, FailedTurnTerminalization,
};
use super::state::RunningTurn;

const PROVIDER_OUTCOME_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const TURN_EVENT_LOOP_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const TURN_TERMINALIZATION_RETRY_LIMIT: usize = 3;
const TURN_TERMINALIZATION_RETRY_BASE_MS: u64 = 50;

mod provider_turns;
mod start_failure;

pub(crate) use provider_turns::{fail_starting_turn, handle_provider_exit, handle_provider_stall};
pub(crate) use start_failure::finalize_start_failure_if_needed;

pub(super) async fn wait_for_turn_event_loop(
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

pub(super) async fn revoke_turn_mcp_token(state: &Arc<AppState>, token: &mut Option<String>) {
    if let Some(token) = token.take() {
        crate::daemon::revoke_provider_session_mcp_token(state.as_ref(), &token).await;
    }
}

pub(super) async fn wait_for_provider_outcome(
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

pub(super) async fn finalize_provider_outcome_required(
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
