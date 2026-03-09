use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{SessionEventType, SessionTurnStatus};
use ctx_providers::adapters::{ProviderAdapter, RunHandle};
use ctx_providers::events::NormalizedEvent;

use crate::daemon::AppState;

use super::persistence::emit_event;
use super::reconcile::{reconcile_turn_failed_on_provider_exit, reconcile_turn_terminal_state};

pub(crate) struct RunningTurn {
    pub(crate) adapter: Arc<dyn ProviderAdapter>,
    pub(crate) handle: RunHandle,
    pub(crate) run_id: RunId,
    pub(crate) turn_id: TurnId,
    pub(crate) event_tx: mpsc::Sender<NormalizedEvent>,
    pub(crate) events_done: Option<oneshot::Receiver<()>>,
}

#[derive(Clone, Copy)]
pub(crate) enum StopReason {
    Cancel,
    Interrupt,
}

impl StopReason {
    fn fallback_reason(self) -> &'static str {
        match self {
            Self::Cancel => "user_cancel",
            Self::Interrupt => "user_interrupt",
        }
    }

    fn should_emit_interrupt_requested(self) -> bool {
        matches!(self, Self::Interrupt)
    }

    pub(crate) fn suspend_queue(self) -> bool {
        matches!(self, Self::Interrupt)
    }
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
                "provider_cancelled": provider_cancelled,
                "status": "interrupted",
            }),
        })
        .await
        .is_ok()
}

pub(crate) async fn stop_running_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn: RunningTurn,
    reason: StopReason,
) -> bool {
    if reason.should_emit_interrupt_requested() {
        let _ = emit_event(
            state,
            session_id,
            Some(turn.run_id),
            Some(turn.turn_id),
            SessionEventType::InterruptRequested,
            json!({"by":"user"}),
        )
        .await;
    }
    let sent = send_turn_interrupted(&turn.event_tx, reason.fallback_reason(), true).await;
    let _ = turn.adapter.cancel(turn.handle).await;
    if !sent {
        let _ = reconcile_turn_terminal_state(
            state,
            session_id,
            Some(turn.run_id),
            turn.turn_id,
            reason.fallback_reason(),
        )
        .await;
    }
    state.set_running(session_id, false).await;
    reason.suspend_queue()
}

pub(crate) async fn handle_provider_exit(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
) {
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    drop(turn.event_tx);
    if let Some(mut events_done) = turn.events_done.take() {
        let state_for_reconcile = Arc::clone(state);
        let events_flushed = tokio::select! {
            _ = &mut events_done => true,
            _ = tokio::time::sleep(Duration::from_secs(2)) => false,
        };
        if events_flushed {
            let _ = reconcile_turn_failed_on_provider_exit(
                &state_for_reconcile,
                session_id,
                Some(run_id),
                turn_id,
                "provider_exit",
            )
            .await;
        } else {
            tracing::debug!(
                session_id = %session_id.0,
                run_id = %run_id.0,
                turn_id = %turn_id.0,
                "event loop still draining after provider exit; deferring reconciliation"
            );
            tokio::spawn(async move {
                tokio::select! {
                    _ = &mut events_done => (),
                    _ = tokio::time::sleep(Duration::from_secs(15)) => (),
                };
                let _ = reconcile_turn_failed_on_provider_exit(
                    &state_for_reconcile,
                    session_id,
                    Some(run_id),
                    turn_id,
                    "provider_exit",
                )
                .await;
            });
        }
    } else {
        let _ = reconcile_turn_failed_on_provider_exit(
            state,
            session_id,
            Some(run_id),
            turn_id,
            "provider_exit",
        )
        .await;
    }
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

    let failed_at = Utc::now();
    let _ = store
        .update_session_turn_status(
            session_id,
            turn_id,
            SessionTurnStatus::Failed,
            None,
            None,
            failed_at,
        )
        .await;
    let _ = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::Error,
        json!({
            "kind": "start_failed",
            "message": error_message,
        }),
    )
    .await;
    let _ = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::TurnFinished,
        json!({
            "message_id": message_id.0,
            "status": "failed",
            "reason": "start_failed",
        }),
    )
    .await;
}
