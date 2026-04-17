use super::head_projection::is_session_gap_notice;
use chrono::Utc;
use ctx_core::ids::{SessionEventId, SessionId};
use ctx_core::models::{SessionEvent, SessionEventType};
use serde_json::json;

fn session_event(event_type: SessionEventType, payload_json: serde_json::Value) -> SessionEvent {
    SessionEvent {
        seq: 1,
        id: SessionEventId(uuid::Uuid::new_v4()),
        session_id: SessionId(uuid::Uuid::new_v4()),
        run_id: None,
        turn_id: None,
        event_type,
        payload_json,
        created_at: Utc::now(),
        transient: false,
    }
}

#[test]
fn detects_session_gap_notice() {
    let event = session_event(
        SessionEventType::Notice,
        json!({
            "kind": "session_gap",
            "reason": "data_plane_overflow",
        }),
    );
    assert!(is_session_gap_notice(&event));
}

#[test]
fn ignores_non_gap_notice() {
    let event = session_event(
        SessionEventType::Notice,
        json!({
            "kind": "context.compacted",
        }),
    );
    assert!(!is_session_gap_notice(&event));
}

#[test]
fn ignores_non_notice_events() {
    let event = session_event(
        SessionEventType::ToolResult,
        json!({
            "kind": "session_gap",
        }),
    );
    assert!(!is_session_gap_notice(&event));
}
