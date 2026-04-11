use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{SessionEventType, SessionTurnStatus};
use ctx_providers::adapters::{ProviderAdapter, RunHandle};
use ctx_providers::events::NormalizedEvent;

use crate::daemon::AppState;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};

use super::interrupt_telemetry::{metric_labels, payload_fields};
use super::persistence::{emit_event, flush_session_events};
use super::reconcile::{reconcile_turn_failed_on_provider_exit, reconcile_turn_terminal_state};
use super::InterruptTelemetryContext;

pub(crate) struct RunningTurn {
    pub(crate) adapter: Arc<dyn ProviderAdapter>,
    pub(crate) handle: RunHandle,
    pub(crate) run_id: RunId,
    pub(crate) turn_id: TurnId,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) execution_environment_label: String,
    pub(crate) session_root_kind: String,
    pub(crate) event_tx: mpsc::Sender<NormalizedEvent>,
    pub(crate) events_done: Option<oneshot::Receiver<()>>,
}

#[derive(Clone, Copy)]
pub(crate) enum StopReason {
    Cancel,
    Interrupt,
    StorageEmergency,
}

impl StopReason {
    fn fallback_reason(self) -> &'static str {
        match self {
            Self::Cancel => "user_cancel",
            Self::Interrupt => "user_interrupt",
            Self::StorageEmergency => "storage_exhausted",
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
    interrupt: Option<&InterruptTelemetryContext>,
) -> bool {
    let mut payload = json!({
        "reason": reason,
        "provider_cancelled": provider_cancelled,
        "status": "interrupted",
    });
    if let Some(ctx) = interrupt {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("interrupt_id".to_string(), json!(ctx.interrupt_id));
            obj.insert(
                "requested_at_ms".to_string(),
                json!(ctx.requested_at_unix_ms()),
            );
        }
    }
    event_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::TurnInterrupted,
            payload_json: payload,
        })
        .await
        .is_ok()
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

pub(crate) async fn stop_running_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn: RunningTurn,
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
    let event_send_started = std::time::Instant::now();
    let sent = send_turn_interrupted(
        &turn.event_tx,
        reason.fallback_reason(),
        true,
        interrupt.as_ref(),
    )
    .await;
    if interrupt.is_some() {
        record_interrupt_metric(
            state,
            &turn,
            "event_send",
            event_send_started.elapsed().as_millis() as u64,
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
    let _ = turn.adapter.cancel(turn.handle).await;
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
    if !sent {
        let _ = reconcile_turn_terminal_state(
            state,
            session_id,
            Some(run_id),
            turn_id,
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
    flush_session_events(&store, session_id, "handle_turn_start_failed").await;
}
