use ctx_core::models::SessionTurnStatus;
use serde_json::Value;

use crate::daemon::scheduler::terminal::{finalize_failed_turn, FailedTurnTerminalization};

use super::state::EventLoopRuntimeState;
use super::telemetry::record_failed_turn_telemetry;
use super::TurnEventLoop;

pub(super) struct TurnFailurePayload {
    pub(super) error_message: String,
    pub(super) details: Option<Value>,
    pub(super) kind: Option<Value>,
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
        state.as_ref(),
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
