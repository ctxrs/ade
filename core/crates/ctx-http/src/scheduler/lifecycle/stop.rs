use std::sync::Arc;

use serde_json::json;

use ctx_core::ids::SessionId;
use ctx_core::models::SessionEventType;
use ctx_providers::adapters::ProviderTurnOutcome;
use ctx_session_tools::interrupt_telemetry::{
    metric_labels, payload_fields, InterruptTelemetryContext,
};

use crate::daemon::AppState;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};

use super::super::persistence::emit_event;
use super::{
    finalize_provider_outcome_required, revoke_turn_mcp_token, wait_for_provider_outcome,
    wait_for_turn_event_loop, RunningTurn, StopReason,
};

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

    fn suspend_queue(self) -> bool {
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
            interrupt_id = %interrupt.interrupt_id(),
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
            interrupt_id = %interrupt.interrupt_id(),
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
    let finalized = if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
        finalize_provider_outcome_required(
            state,
            session_id,
            Some(run_id),
            turn_id,
            turn.message_id,
            outcome,
        )
        .await
    } else {
        finalize_provider_outcome_required(
            state,
            session_id,
            Some(run_id),
            turn_id,
            turn.message_id,
            outcome,
        )
        .await
    };
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
    state.set_running(session_id, false).await;
    reason.suspend_queue() || !finalized
}
