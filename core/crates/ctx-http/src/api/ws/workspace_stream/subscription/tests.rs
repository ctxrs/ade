use super::*;

fn cursor(last_event_seq: i64, projection_rev: i64) -> SessionCursor {
    SessionCursor {
        last_sent: SessionReplayCursor {
            last_event_seq,
            projection_rev,
        },
    }
}

#[test]
fn merge_replayed_and_live_subscriptions_keeps_live_only_sessions_and_drops_removed_sessions() {
    let replayed_session_id = SessionId::new();
    let live_only_session_id = SessionId::new();
    let removed_session_id = SessionId::new();
    let live_subscriptions = HashMap::from([
        (replayed_session_id, cursor(15, 15)),
        (live_only_session_id, cursor(7, 7)),
    ]);
    let replayed_subscriptions = HashMap::from([
        (replayed_session_id, cursor(12, 12)),
        (removed_session_id, cursor(20, 20)),
    ]);

    let merged = merge_replayed_and_live_subscriptions(&live_subscriptions, replayed_subscriptions);

    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged
            .get(&replayed_session_id)
            .map(|subscription| subscription.last_sent),
        Some(SessionReplayCursor {
            last_event_seq: 15,
            projection_rev: 15,
        })
    );
    assert_eq!(
        merged
            .get(&live_only_session_id)
            .map(|subscription| subscription.last_sent),
        Some(SessionReplayCursor {
            last_event_seq: 7,
            projection_rev: 7,
        })
    );
    assert!(
        !merged.contains_key(&removed_session_id),
        "live subscription state must remain authoritative for removed sessions",
    );
}
