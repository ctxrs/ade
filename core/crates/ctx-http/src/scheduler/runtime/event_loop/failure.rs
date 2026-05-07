use std::collections::HashMap;

use ctx_core::models::SessionTurnStatus;
use serde_json::{json, Value};

use crate::ops_events::OpsEvent;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::scheduler::terminal::{finalize_failed_turn, FailedTurnTerminalization};
use crate::telemetry::TelemetryEvent;

use super::state::EventLoopRuntimeState;
use super::TurnEventLoop;

pub(super) struct TurnFailurePayload {
    pub(super) error_message: String,
    pub(super) details: Option<Value>,
    pub(super) kind: Option<Value>,
}

pub(super) async fn record_failed_turn_telemetry(
    ctx: &TurnEventLoop,
    runtime: &mut EventLoopRuntimeState,
    error_message: String,
    details: Option<Value>,
    kind: Option<Value>,
) {
    let Some(state) = ctx.state() else {
        return;
    };
    if !runtime.telemetry_emitted {
        runtime.telemetry_emitted = true;
        let duration_ms = ctx.run_started_at.elapsed().as_millis() as u64;
        let mut run_labels = HashMap::new();
        run_labels.insert("provider_id".to_string(), ctx.provider_id.clone());
        run_labels.insert("model_id".to_string(), ctx.model_id.clone());
        run_labels.insert(
            "execution_environment".to_string(),
            ctx.execution_environment_label.clone(),
        );
        run_labels.insert(
            "session_root_kind".to_string(),
            ctx.session_root_kind.clone(),
        );
        run_labels.insert("event".to_string(), "run_failed".to_string());
        let run_metric = PerfMetric {
            name: "scheduler.run_total_ms".to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: duration_ms as f64,
            labels: run_labels,
        };
        state
            .telemetry
            .perf_telemetry
            .record_metric(run_metric, ctx.perf_run_id.clone(), None, None)
            .await;
        state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::provider_call(
                ctx.provider_id.clone(),
                ctx.model_id.clone(),
                Some(ctx.execution_environment_label.clone()),
                Some(ctx.session_root_kind.clone()),
                false,
                duration_ms,
            ))
            .await;
        state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::session_completed(
                ctx.provider_id.clone(),
                ctx.model_id.clone(),
                Some(ctx.execution_environment_label.clone()),
                Some(ctx.session_root_kind.clone()),
                "failed".to_string(),
                duration_ms,
            ))
            .await;
    }

    let mut fail_event = OpsEvent::new("error", "provider_run_failed");
    fail_event.session_id = Some(ctx.session_id.0.to_string());
    fail_event.worktree_id = Some(ctx.worktree_id.0.to_string());
    fail_event.run_id = Some(ctx.run_id.0.to_string());
    fail_event.turn_id = Some(ctx.turn_id.0.to_string());
    fail_event.provider_id = Some(ctx.provider_id.clone());
    fail_event.cwd = Some(ctx.workdir_str.clone());
    fail_event.worktree_root = Some(ctx.workdir_str.clone());
    fail_event.meta = Some(json!({
        "model_id": ctx.model_id.clone(),
        "execution_environment": ctx.execution_environment_label.clone(),
        "session_root_kind": ctx.session_root_kind.clone(),
        "error": error_message.clone(),
        "details": details.clone(),
        "kind": kind.clone(),
    }));
    state.telemetry.ops_events.emit(fail_event);
}

pub(super) async fn fail_turn(
    ctx: &TurnEventLoop,
    runtime: &mut EventLoopRuntimeState,
    failure: TurnFailurePayload,
    emit_error_event: bool,
) {
    let Some(state) = ctx.state() else {
        return;
    };
    record_failed_turn_telemetry(
        ctx,
        runtime,
        failure.error_message.clone(),
        failure.details.clone(),
        failure.kind.clone(),
    )
    .await;
    runtime.terminal_status = Some(SessionTurnStatus::Failed);
    let _ = finalize_failed_turn(
        &state,
        ctx.session_id,
        Some(ctx.run_id),
        ctx.turn_id,
        ctx.message_id,
        FailedTurnTerminalization {
            message: &failure.error_message,
            reason: None,
            details: failure.details,
            kind: failure.kind,
            emit_error_event,
        },
    )
    .await;
}
