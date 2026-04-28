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
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};

use super::interrupt_telemetry::{metric_labels, payload_fields};
use super::persistence::emit_event;
use super::reconcile::reconcile_turn_terminal_state;
use super::terminal::{finalize_failed_turn, finalize_provider_outcome, FailedTurnTerminalization};
use super::InterruptTelemetryContext;

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

impl StopReason {
    fn should_emit_interrupt_requested(self) -> bool {
        matches!(self, Self::Interrupt)
    }

    fn missing_outcome_reason(self) -> &'static str {
        match self {
            Self::Cancel => "user_cancel_missing_outcome",
            Self::Interrupt => "user_interrupt_missing_outcome",
            Self::StorageEmergency => "storage_exhausted_missing_outcome",
        }
    }

    fn outcome_timeout_reason(self) -> &'static str {
        match self {
            Self::Cancel => "user_cancel_outcome_timeout",
            Self::Interrupt => "user_interrupt_outcome_timeout",
            Self::StorageEmergency => "storage_exhausted_outcome_timeout",
        }
    }

    pub(crate) fn suspend_queue(self) -> bool {
        matches!(self, Self::Interrupt)
    }
}

async fn record_interrupt_metric(
    state: &Arc<AppState>,
    turn: &RunningTurn,
    event: &str,
    value_ms: u64,
) {
    let metric = PerfMetric {
        name: "scheduler.interrupt_latency_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: value_ms as f64,
        labels: metric_labels(
            &turn.provider_id,
            &turn.model_id,
            &turn.execution_environment_label,
            &turn.session_root_kind,
            event,
        ),
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(metric, Some(turn.run_id.0.to_string()), None, None)
        .await;
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

fn interrupted_fallback_outcome(reason: &str, provider_cancelled: bool) -> ProviderTurnOutcome {
    ProviderTurnOutcome {
        terminal_event_emitted: false,
        ..ProviderTurnOutcome::interrupted(reason, provider_cancelled)
    }
}

pub(crate) async fn stop_running_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
    reason: StopReason,
    interrupt: Option<InterruptTelemetryContext>,
) -> bool {
    if matches!(reason, StopReason::StorageEmergency) {
        let _ = emit_event(
            state,
            session_id,
            Some(turn.run_id),
            Some(turn.turn_id),
            SessionEventType::Notice,
            json!({
                "kind": "storage_guard_kill",
                "message": "Storage emergency interrupted this session to protect local data.",
            }),
        )
        .await;
    }
    if let Some(interrupt) = interrupt.as_ref() {
        record_interrupt_metric(state, &turn, "request_age", interrupt.elapsed_ms()).await;
        tracing::info!(
            session_id = %session_id.0,
            run_id = %turn.run_id.0,
            turn_id = %turn.turn_id.0,
            interrupt_id = %interrupt.interrupt_id,
            provider_id = %turn.provider_id,
            model_id = %turn.model_id,
            request_age_ms = interrupt.elapsed_ms(),
            "session interrupt requested"
        );
    }
    if reason.should_emit_interrupt_requested() {
        let mut payload = json!({"by":"user"});
        if let Some(interrupt) = interrupt.as_ref() {
            if let Some(obj) = payload.as_object_mut() {
                let extra = payload_fields(interrupt);
                if let Some(extra_obj) = extra.as_object() {
                    for (key, value) in extra_obj {
                        obj.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        let _ = emit_event(
            state,
            session_id,
            Some(turn.run_id),
            Some(turn.turn_id),
            SessionEventType::InterruptRequested,
            payload,
        )
        .await;
    }
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    let provider_id = turn.provider_id.clone();
    let model_id = turn.model_id.clone();
    let execution_environment_label = turn.execution_environment_label.clone();
    let session_root_kind = turn.session_root_kind.clone();
    let cancel_started = std::time::Instant::now();
    let _ = turn.adapter.cancel(&mut turn.handle).await;
    if let Some(interrupt) = interrupt.as_ref() {
        let cancel_ms = cancel_started.elapsed().as_millis() as u64;
        let metric = PerfMetric {
            name: "scheduler.interrupt_latency_ms".to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: cancel_ms as f64,
            labels: metric_labels(
                &provider_id,
                &model_id,
                &execution_environment_label,
                &session_root_kind,
                "provider_cancel",
            ),
        };
        state
            .telemetry
            .perf_telemetry
            .record_metric(metric, Some(run_id.0.to_string()), None, None)
            .await;
        tracing::info!(
            session_id = %session_id.0,
            run_id = %run_id.0,
            turn_id = %turn_id.0,
            interrupt_id = %interrupt.interrupt_id,
            provider_cancel_ms = cancel_ms,
            interrupt_total_ms = interrupt.elapsed_ms(),
            "session interrupt provider cancel finished"
        );
    }
    let outcome = wait_for_provider_outcome(
        &mut turn.handle,
        interrupted_fallback_outcome(reason.missing_outcome_reason(), false),
        interrupted_fallback_outcome(reason.outcome_timeout_reason(), false),
    )
    .await;
    drop(turn.event_tx);
    if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
        let _ = finalize_provider_outcome(
            state,
            session_id,
            Some(run_id),
            turn_id,
            turn.message_id,
            outcome,
        )
        .await;
    } else {
        let _ = finalize_provider_outcome(
            state,
            session_id,
            Some(run_id),
            turn_id,
            turn.message_id,
            outcome,
        )
        .await;
    }
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
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
    if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
        let _ = finalize_provider_outcome(
            state,
            session_id,
            Some(run_id),
            turn_id,
            message_id,
            outcome,
        )
        .await;
    } else {
        let _ = finalize_provider_outcome(
            state,
            session_id,
            Some(run_id),
            turn_id,
            message_id,
            outcome,
        )
        .await;
    }
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
}

pub(crate) async fn handle_provider_stall(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
) {
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
    let _ = finalize_provider_outcome(
        state,
        session_id,
        Some(run_id),
        turn_id,
        message_id,
        outcome,
    )
    .await;
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
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
