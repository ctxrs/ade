use std::collections::HashMap;

use ctx_core::models::{SessionEvent, SessionEventType, SessionTurnStatus};
use ctx_session_tools::interrupt_telemetry::{latency_bucket, metric_labels};
use serde_json::Value;

use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::telemetry::TelemetryEvent;

use super::failure::record_failed_turn_telemetry;
use super::state::EventLoopRuntimeState;
use super::TurnEventLoop;

pub(super) fn is_truthful_start_activity(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::TurnStarted
            | SessionEventType::AssistantChunk
            | SessionEventType::ThoughtChunk
            | SessionEventType::AssistantComplete
            | SessionEventType::ContextWindowUpdate
            | SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult
            | SessionEventType::Done
            | SessionEventType::TurnInterrupted
            | SessionEventType::Error
    )
}

pub(super) async fn handle_session_gap_notice(ctx: &TurnEventLoop, event: &SessionEvent) {
    let Some(state) = ctx.state() else {
        return;
    };
    let reason = event
        .payload_json
        .get("reason")
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    state
        .workspaces
        .workspace_active_snapshot
        .publish_session_gap(ctx.workspace_id, ctx.session_id, event.seq, reason)
        .await;
}

pub(super) async fn handle_done_event(ctx: &TurnEventLoop, runtime: &mut EventLoopRuntimeState) {
    let Some(state) = ctx.state() else {
        return;
    };
    runtime.promote_terminal(&ctx.start_progress_tx);
    if runtime.terminal_status.is_some() {
        return;
    }
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
        run_labels.insert("event".to_string(), "run_complete".to_string());
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
                true,
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
                "completed".to_string(),
                duration_ms,
            ))
            .await;
    }
    let _ = ctx
        .store
        .delete_session_events_for_turn_types(
            ctx.session_id,
            ctx.turn_id,
            &[
                SessionEventType::ThoughtChunk,
                SessionEventType::ContextWindowUpdate,
            ],
        )
        .await;
    runtime.terminal_status = Some(SessionTurnStatus::Completed);
}

pub(super) async fn handle_turn_interrupted(
    ctx: &TurnEventLoop,
    runtime: &mut EventLoopRuntimeState,
    event: &SessionEvent,
) {
    let Some(state) = ctx.state() else {
        return;
    };
    runtime.promote_terminal(&ctx.start_progress_tx);
    if runtime.terminal_status.is_some() {
        return;
    }
    if !runtime.telemetry_emitted {
        runtime.telemetry_emitted = true;
        let duration_ms = ctx.run_started_at.elapsed().as_millis() as u64;
        let run_metric = PerfMetric {
            name: "scheduler.run_total_ms".to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: duration_ms as f64,
            labels: metric_labels(
                &ctx.provider_id,
                &ctx.model_id,
                &ctx.execution_environment_label,
                &ctx.session_root_kind,
                "run_interrupt",
            ),
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
                "interrupted".to_string(),
                duration_ms,
            ))
            .await;
    }
    if let Some(requested_at_ms) = event
        .payload_json
        .get("requested_at_ms")
        .and_then(Value::as_i64)
    {
        let latency_ms = (event.created_at.timestamp_millis() - requested_at_ms).max(0) as u64;
        let bucket = latency_bucket(latency_ms).to_string();
        let interrupt_metric = PerfMetric {
            name: "scheduler.interrupt_total_ms".to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: latency_ms as f64,
            labels: metric_labels(
                &ctx.provider_id,
                &ctx.model_id,
                &ctx.execution_environment_label,
                &ctx.session_root_kind,
                "turn_interrupted_visible",
            ),
        };
        state
            .telemetry
            .perf_telemetry
            .record_metric(interrupt_metric, ctx.perf_run_id.clone(), None, None)
            .await;
        state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::session_interrupt_latency(
                ctx.provider_id.clone(),
                ctx.model_id.clone(),
                Some(ctx.execution_environment_label.clone()),
                Some(ctx.session_root_kind.clone()),
                latency_ms,
                bucket.clone(),
            ))
            .await;
        let interrupt_id = event
            .payload_json
            .get("interrupt_id")
            .and_then(Value::as_str);
        tracing::info!(
            session_id = %ctx.session_id.0,
            run_id = %ctx.run_id.0,
            turn_id = %ctx.turn_id.0,
            interrupt_id,
            interrupt_total_ms = latency_ms,
            duration_bucket = %bucket,
            "session interrupt became visible in event loop"
        );
    }
    runtime.terminal_status = Some(SessionTurnStatus::Interrupted);
    let _ = ctx
        .store
        .delete_session_events_for_turn_types(
            ctx.session_id,
            ctx.turn_id,
            &[
                SessionEventType::AssistantChunk,
                SessionEventType::ThoughtChunk,
                SessionEventType::ContextWindowUpdate,
            ],
        )
        .await;
}

pub(super) async fn handle_error_event(
    ctx: &TurnEventLoop,
    runtime: &mut EventLoopRuntimeState,
    event: &SessionEvent,
) {
    if ctx.state().is_none() {
        return;
    }
    runtime.promote_terminal(&ctx.start_progress_tx);
    if runtime.terminal_status.is_some() {
        return;
    }
    let error_message = event
        .payload_json
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("provider runtime error")
        .to_string();
    record_failed_turn_telemetry(
        ctx,
        runtime,
        error_message,
        event.payload_json.get("details").cloned(),
        event.payload_json.get("kind").cloned(),
    )
    .await;
    runtime.terminal_status = Some(SessionTurnStatus::Failed);
    let _ = ctx
        .store
        .delete_session_events_for_turn_types(
            ctx.session_id,
            ctx.turn_id,
            &[
                SessionEventType::AssistantChunk,
                SessionEventType::ThoughtChunk,
                SessionEventType::ContextWindowUpdate,
            ],
        )
        .await;
}
