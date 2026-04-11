use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::models::{SessionActivityState, SessionEvent, SessionEventType, SessionTurnStatus};

#[derive(Debug, Clone, PartialEq)]
pub struct TurnTerminalState {
    pub status: SessionTurnStatus,
    pub end_seq: Option<i64>,
    pub metrics: Option<Value>,
    pub updated_at: DateTime<Utc>,
}

pub fn derive_activity_from_status(
    last_status: Option<SessionTurnStatus>,
    has_running_turn: bool,
) -> SessionActivityState {
    SessionActivityState {
        is_working: has_running_turn,
        last_turn_status: last_status,
    }
}

pub fn turn_status_from_finished_payload(payload: &Value) -> Option<SessionTurnStatus> {
    match payload.get("status").and_then(Value::as_str) {
        Some("completed") => Some(SessionTurnStatus::Completed),
        Some("failed" | "error") => Some(SessionTurnStatus::Failed),
        Some("interrupted") => Some(SessionTurnStatus::Interrupted),
        Some("queued") => Some(SessionTurnStatus::Queued),
        Some("running") => Some(SessionTurnStatus::Running),
        _ => None,
    }
}

pub fn turn_status_from_event(event: &SessionEvent) -> Option<SessionTurnStatus> {
    match event.event_type {
        SessionEventType::TurnQueued => Some(SessionTurnStatus::Queued),
        SessionEventType::TurnStarted => Some(SessionTurnStatus::Running),
        SessionEventType::Done => Some(SessionTurnStatus::Completed),
        SessionEventType::Error => Some(SessionTurnStatus::Failed),
        SessionEventType::TurnInterrupted => Some(SessionTurnStatus::Interrupted),
        SessionEventType::TurnFinished => turn_status_from_finished_payload(&event.payload_json),
        _ => None,
    }
}

fn is_terminal_event(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::Done
            | SessionEventType::Error
            | SessionEventType::TurnInterrupted
            | SessionEventType::TurnFinished
    )
}

fn latest_done_metrics(events: &[SessionEvent]) -> Option<Value> {
    events
        .iter()
        .rev()
        .find(|event| matches!(event.event_type, SessionEventType::Done))
        .and_then(|event| event.payload_json.get("context_window"))
        .cloned()
}

pub fn resolve_turn_terminal_state(events: &[SessionEvent]) -> Option<TurnTerminalState> {
    let terminal_index = events
        .iter()
        .rposition(|event| is_terminal_event(&event.event_type))?;
    let event = &events[terminal_index];

    match event.event_type {
        SessionEventType::Done => Some(TurnTerminalState {
            status: SessionTurnStatus::Completed,
            end_seq: Some(event.seq),
            metrics: event.payload_json.get("context_window").cloned(),
            updated_at: event.created_at,
        }),
        SessionEventType::Error => Some(TurnTerminalState {
            status: SessionTurnStatus::Failed,
            end_seq: None,
            metrics: None,
            updated_at: event.created_at,
        }),
        SessionEventType::TurnInterrupted => Some(TurnTerminalState {
            status: SessionTurnStatus::Interrupted,
            end_seq: Some(event.seq),
            metrics: None,
            updated_at: event.created_at,
        }),
        SessionEventType::TurnFinished => {
            let status =
                turn_status_from_finished_payload(&event.payload_json).unwrap_or_else(|| {
                    events[..terminal_index]
                        .iter()
                        .rev()
                        .find_map(turn_status_from_event)
                        .unwrap_or(SessionTurnStatus::Completed)
                });
            let metrics = if status == SessionTurnStatus::Completed {
                latest_done_metrics(&events[..=terminal_index])
            } else {
                None
            };
            Some(TurnTerminalState {
                status,
                end_seq: Some(event.seq),
                metrics,
                updated_at: event.created_at,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use crate::ids::{SessionEventId, SessionId, TurnId};
    use crate::models::{SessionEvent, SessionEventType, SessionTurnStatus};

    use super::{resolve_turn_terminal_state, turn_status_from_finished_payload};

    fn event(
        seq: i64,
        turn_id: TurnId,
        event_type: SessionEventType,
        payload_json: serde_json::Value,
    ) -> SessionEvent {
        SessionEvent {
            seq,
            id: SessionEventId::new(),
            session_id: SessionId::new(),
            run_id: None,
            turn_id: Some(turn_id),
            event_type,
            payload_json,
            transient: false,
            created_at: Utc.timestamp_opt(seq, 0).unwrap(),
        }
    }

    #[test]
    fn turn_finished_status_parses_known_values() {
        assert_eq!(
            turn_status_from_finished_payload(&json!({ "status": "completed" })),
            Some(SessionTurnStatus::Completed)
        );
        assert_eq!(
            turn_status_from_finished_payload(&json!({ "status": "failed" })),
            Some(SessionTurnStatus::Failed)
        );
        assert_eq!(
            turn_status_from_finished_payload(&json!({ "status": "interrupted" })),
            Some(SessionTurnStatus::Interrupted)
        );
    }

    #[test]
    fn resolve_terminal_state_prefers_last_terminal_event() {
        let turn_id = TurnId::new();
        let events = vec![
            event(1, turn_id, SessionEventType::TurnStarted, json!({})),
            event(
                2,
                turn_id,
                SessionEventType::Done,
                json!({ "context_window": { "total_tokens": 42 } }),
            ),
            event(
                3,
                turn_id,
                SessionEventType::TurnFinished,
                json!({ "status": "completed" }),
            ),
        ];
        let terminal = resolve_turn_terminal_state(&events).expect("terminal state");
        assert_eq!(terminal.status, SessionTurnStatus::Completed);
        assert_eq!(terminal.end_seq, Some(3));
        assert_eq!(terminal.metrics, Some(json!({ "total_tokens": 42 })));
    }
}
