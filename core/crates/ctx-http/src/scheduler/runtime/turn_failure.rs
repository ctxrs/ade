use std::sync::Arc;

use serde_json::json;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::Session;

use crate::daemon::AppState;
use crate::scheduler::terminal::{finalize_failed_turn, FailedTurnTerminalization};

pub(super) async fn emit_turn_start_failed(
    state: &Arc<AppState>,
    session: &Session,
    run_id: RunId,
    turn_id: TurnId,
    message_id: MessageId,
    err: &anyhow::Error,
) {
    let error_message = err.to_string();
    let _ = finalize_failed_turn(
        state,
        session.id,
        Some(run_id),
        turn_id,
        message_id,
        FailedTurnTerminalization {
            message: &error_message,
            reason: Some("start_failed"),
            details: None,
            kind: Some(json!("start_failed")),
            emit_error_event: true,
        },
    )
    .await;
}
