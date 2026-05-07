use ctx_core::models::{SessionEvent, SessionEventType, SessionTurnStatus};
use serde_json::Value;

use super::state::EventLoopRuntimeState;
use super::telemetry::{
    record_failed_turn_telemetry, record_interrupt_visible_telemetry, record_terminal_run_telemetry,
};
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
    record_terminal_run_telemetry(
        ctx,
        runtime,
        state.as_ref(),
        "run_complete",
        true,
        "completed",
    )
    .await;
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
    record_terminal_run_telemetry(
        ctx,
        runtime,
        state.as_ref(),
        "run_interrupt",
        false,
        "interrupted",
    )
    .await;
    record_interrupt_visible_telemetry(ctx, state.as_ref(), event).await;
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
    let Some(state) = ctx.state() else {
        return;
    };
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
        state.as_ref(),
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
