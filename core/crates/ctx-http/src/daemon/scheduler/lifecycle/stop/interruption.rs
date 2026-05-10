use std::sync::Arc;

use serde_json::json;

use ctx_core::ids::SessionId;
use ctx_core::models::SessionEventType;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_providers::adapters::ProviderTurnOutcome;
use ctx_session_tools::interrupt_telemetry::{
    metric_labels, payload_fields, InterruptTelemetryContext,
};

use crate::daemon::AppState;

use super::super::super::persistence::emit_event;
use super::super::state::{RunningTurn, StopReason};

impl StopReason {
    pub(super) fn should_emit_interrupt_requested(self) -> bool {
        matches!(self, Self::Interrupt)
    }

    pub(super) fn missing_outcome_reason(self) -> &'static str {
        match self {
            Self::Cancel => "user_cancel_missing_outcome",
            Self::Interrupt => "user_interrupt_missing_outcome",
            Self::StorageEmergency => "storage_exhausted_missing_outcome",
        }
    }

    pub(super) fn outcome_timeout_reason(self) -> &'static str {
        match self {
            Self::Cancel => "user_cancel_outcome_timeout",
            Self::Interrupt => "user_interrupt_outcome_timeout",
            Self::StorageEmergency => "storage_exhausted_outcome_timeout",
        }
    }

    pub(super) fn suspend_queue(self) -> bool {
        matches!(self, Self::Interrupt)
    }
}

pub(super) async fn record_interrupt_request_telemetry(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn: &RunningTurn,
    interrupt: &InterruptTelemetryContext,
) {
    record_interrupt_metric(state, turn, "request_age", interrupt.elapsed_ms()).await;
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

pub(super) async fn emit_interrupt_requested_event(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn: &RunningTurn,
    interrupt: Option<&InterruptTelemetryContext>,
) {
    let mut payload = json!({"by":"user"});
    if let Some(interrupt) = interrupt {
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

pub(super) async fn record_provider_cancel_telemetry(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn: &RunningTurn,
    interrupt: &InterruptTelemetryContext,
    cancel_ms: u64,
) {
    record_interrupt_metric(state, turn, "provider_cancel", cancel_ms).await;
    tracing::info!(
        session_id = %session_id.0,
        run_id = %turn.run_id.0,
        turn_id = %turn.turn_id.0,
        interrupt_id = %interrupt.interrupt_id(),
        provider_cancel_ms = cancel_ms,
        interrupt_total_ms = interrupt.elapsed_ms(),
        "session interrupt provider cancel finished"
    );
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

pub(super) fn interrupted_fallback_outcome(
    reason: &str,
    provider_cancelled: bool,
) -> ProviderTurnOutcome {
    ProviderTurnOutcome {
        terminal_event_emitted: false,
        ..ProviderTurnOutcome::interrupted(reason, provider_cancelled)
    }
}
