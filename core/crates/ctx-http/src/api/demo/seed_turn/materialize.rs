use serde_json::Value;

use super::records::SeededTurn;
use super::{internal_error, SeedTurnResult};
use ctx_core::models::{SessionTurn, SessionTurnStatus};
use ctx_store::Store;

pub(super) async fn insert_user_message(store: &Store, seeded: &SeededTurn) -> SeedTurnResult<()> {
    store
        .insert_message(seeded.user_message.clone())
        .await
        .map(|_| ())
        .map_err(|_| internal_error("failed to insert user message"))
}

pub(super) async fn insert_assistant_message(
    store: &Store,
    seeded: &SeededTurn,
) -> SeedTurnResult<()> {
    store
        .insert_message(seeded.assistant_message.clone())
        .await
        .map(|_| ())
        .map_err(|_| internal_error("failed to insert assistant message"))
}

pub(super) async fn insert_session_turn(
    store: &Store,
    seeded: &SeededTurn,
    metrics_json: Option<Value>,
    start_seq: i64,
    end_seq: i64,
) -> SeedTurnResult<()> {
    store
        .insert_session_turn(build_session_turn(seeded, metrics_json, start_seq, end_seq))
        .await
        .map(|_| ())
        .map_err(|_| internal_error("failed to insert session turn"))
}

fn build_session_turn(
    seeded: &SeededTurn,
    metrics_json: Option<Value>,
    start_seq: i64,
    end_seq: i64,
) -> SessionTurn {
    SessionTurn {
        turn_id: seeded.turn_id,
        session_id: seeded.session_id,
        run_id: Some(seeded.run_id),
        user_message_id: Some(seeded.user_message.id),
        status: SessionTurnStatus::Completed,
        start_seq: Some(start_seq),
        end_seq: Some(end_seq),
        started_at: seeded.user_created_at,
        updated_at: seeded.assistant_created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json,
        failure: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::*;
    use crate::api::demo::types::SeedTranscriptTurnReq;
    use ctx_core::ids::{SessionId, TaskId};

    #[test]
    fn build_session_turn_uses_explicit_user_and_done_bounds() {
        let turn = SeedTranscriptTurnReq {
            user: "user".to_string(),
            assistant: "assistant".to_string(),
            context_window: Some(json!({"total_tokens": 42})),
        };
        let seeded = SeededTurn::new(
            SessionId::new(),
            TaskId::new(),
            2,
            Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
                .single()
                .expect("base time"),
            &turn,
        );

        let session_turn = build_session_turn(&seeded, turn.context_window.clone(), 101, 104);

        assert_eq!(session_turn.start_seq, Some(101));
        assert_eq!(session_turn.end_seq, Some(104));
        assert_eq!(session_turn.user_message_id, Some(seeded.user_message.id));
        assert_eq!(session_turn.metrics_json, Some(json!({"total_tokens": 42})));
    }
}
