use serde_json::{json, Value};

use ctx_core::ids::MessageId;
use ctx_core::models::SessionEventType;

use super::super::types::{FailedTurnTerminalization, InterruptedTurnTerminalization};

pub(super) fn completed_turn_finished_event(message_id: MessageId) -> (SessionEventType, Value) {
    (
        SessionEventType::TurnFinished,
        json!({
            "message_id": message_id.0,
            "status": "completed",
        }),
    )
}

pub(super) fn interrupted_turn_events(
    message_id: MessageId,
    interruption: InterruptedTurnTerminalization<'_>,
) -> Vec<(SessionEventType, Value)> {
    let mut events = Vec::new();
    if interruption.emit_interrupt_event {
        events.push((
            SessionEventType::TurnInterrupted,
            json!({
                "reason": interruption.reason,
                "provider_cancelled": interruption.provider_cancelled,
                "status": "interrupted",
            }),
        ));
    }
    events.push((
        SessionEventType::TurnFinished,
        json!({
            "message_id": message_id.0,
            "status": "interrupted",
            "reason": interruption.reason,
            "provider_cancelled": interruption.provider_cancelled,
        }),
    ));
    events
}

pub(super) fn failed_turn_finished_event(
    message_id: MessageId,
    failure: FailedTurnTerminalization<'_>,
) -> (SessionEventType, Value) {
    let mut finished = json!({
        "message_id": message_id.0,
        "status": "failed",
    });
    if let Some(obj) = finished.as_object_mut() {
        if let Some(reason) = failure.reason {
            obj.insert("reason".to_string(), json!(reason));
        }
        obj.insert("message".to_string(), json!(failure.message));
        if let Some(details) = failure.details {
            obj.insert("details".to_string(), details);
        }
        if let Some(kind) = failure.kind {
            obj.insert("kind".to_string(), kind);
        }
    }
    (SessionEventType::TurnFinished, finished)
}
