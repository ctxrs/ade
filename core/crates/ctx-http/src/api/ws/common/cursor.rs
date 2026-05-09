use super::super::*;

pub(in crate::api::ws) struct SessionCursor {
    pub(in crate::api::ws) last_sent: SessionReplayCursor,
}

pub(in crate::api::ws) fn accept_session_delta(
    cursor: &mut SessionCursor,
    delta: &SessionHeadDelta,
) -> bool {
    if is_transient_session_delta(delta) {
        return true;
    }
    let incoming = SessionReplayCursor::from_delta(delta);
    accept_session_cursor(cursor, incoming)
}

pub(in crate::api::ws) fn accept_session_head(
    cursor: &mut SessionCursor,
    head: &SessionHeadSnapshot,
) -> bool {
    accept_session_cursor(cursor, SessionReplayCursor::from_head(head))
}

fn accept_session_cursor(cursor: &mut SessionCursor, incoming: SessionReplayCursor) -> bool {
    if incoming <= cursor.last_sent {
        return false;
    }
    cursor.last_sent = incoming;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn transient_delta(session_id: SessionId, turn_id: TurnId) -> SessionHeadDelta {
        SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 7,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: Some(SessionEvent {
                seq: -1,
                id: SessionEventId::new(),
                session_id,
                run_id: None,
                turn_id: Some(turn_id),
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({ "content_fragment": "partial" }),
                transient: true,
                created_at: Utc::now(),
            }),
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        }
    }

    #[test]
    fn transient_delta_is_never_dropped_by_cursor_dedup() {
        let mut cursor = SessionCursor {
            last_sent: SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            },
        };

        assert!(accept_session_delta(
            &mut cursor,
            &transient_delta(SessionId::new(), TurnId::new())
        ));
        assert_eq!(
            cursor.last_sent,
            SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            }
        );
    }

    #[test]
    fn stale_durable_delta_is_dropped() {
        let mut cursor = SessionCursor {
            last_sent: SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            },
        };
        let delta = SessionHeadDelta {
            session_id: SessionId::new(),
            last_event_seq: 5,
            projection_rev: 7,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };

        assert!(!accept_session_delta(&mut cursor, &delta));
    }

    #[test]
    fn newer_durable_delta_advances_cursor() {
        let mut cursor = SessionCursor {
            last_sent: SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 7,
            },
        };
        let delta = SessionHeadDelta {
            session_id: SessionId::new(),
            last_event_seq: 5,
            projection_rev: 8,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };

        assert!(accept_session_delta(&mut cursor, &delta));
        assert_eq!(
            cursor.last_sent,
            SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 8,
            }
        );
    }
}
