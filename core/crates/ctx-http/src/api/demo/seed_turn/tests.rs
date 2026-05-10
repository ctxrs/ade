use chrono::{TimeZone, Utc};
use serde_json::json;

use super::*;
use crate::api::demo::types::SeedTranscriptTurnReq;
use ctx_core::models::{
    ExecutionEnvironment, MessageRole, SessionEventType, SessionTurnStatus, VcsKind,
};

async fn setup_session() -> (tempfile::TempDir, Store, SessionId, TaskId) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path().join("db.sqlite"))
        .await
        .expect("open store");
    let workspace = store
        .create_workspace(
            "seed-demo".to_string(),
            dir.path().join("repo").display().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let task = store
        .create_task(workspace.id, "seed transcript".to_string(), None)
        .await
        .expect("create task");
    let worktree = store
        .create_worktree(
            workspace.id,
            dir.path().join("worktree").display().to_string(),
            "abc123".to_string(),
            None,
        )
        .await
        .expect("create worktree");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "fake".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create session");

    (dir, store, session.id, task.id)
}

#[tokio::test]
async fn seed_turn_preserves_tail_materialization_event_order_and_bounds() {
    let (_dir, store, session_id, task_id) = setup_session().await;
    let base_time = Utc
        .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
        .single()
        .expect("base time");
    let skipped_head = SeedTranscriptTurnReq {
        user: "head user".to_string(),
        assistant: "head assistant".to_string(),
        context_window: Some(json!({"total_tokens": 111})),
    };
    let materialized_tail = SeedTranscriptTurnReq {
        user: "tail user".to_string(),
        assistant: "tail assistant".to_string(),
        context_window: Some(json!({"total_tokens": 222})),
    };

    let skipped_counts = seed_transcript_turn(
        &store,
        session_id,
        task_id,
        0,
        base_time,
        &skipped_head,
        false,
    )
    .await
    .expect("seed skipped head");
    let tail_counts = seed_transcript_turn(
        &store,
        session_id,
        task_id,
        1,
        base_time,
        &materialized_tail,
        true,
    )
    .await
    .expect("seed materialized tail");

    assert_eq!(skipped_counts.messages, 0);
    assert_eq!(skipped_counts.events, 5);
    assert_eq!(tail_counts.messages, 2);
    assert_eq!(tail_counts.events, 5);

    let messages = store
        .list_messages_for_session(session_id)
        .await
        .expect("list messages");
    assert_eq!(messages.len(), 2);
    assert!(matches!(&messages[0].role, MessageRole::User));
    assert!(matches!(&messages[1].role, MessageRole::Assistant));
    assert_eq!(messages[0].content, "tail user");
    assert_eq!(messages[1].content, "tail assistant");
    assert_eq!(messages[0].turn_sequence, Some(0));
    assert_eq!(messages[1].turn_sequence, Some(1));
    assert_eq!(messages[0].order_seq, Some(3));
    assert_eq!(messages[1].order_seq, Some(4));

    let events = store
        .list_session_events(session_id)
        .await
        .expect("list events");
    assert_eq!(events.len(), 10);
    assert!(matches!(
        &events[0].event_type,
        SessionEventType::UserMessage
    ));
    assert!(matches!(
        &events[1].event_type,
        SessionEventType::TurnStarted
    ));
    assert!(matches!(
        &events[2].event_type,
        SessionEventType::AssistantMessageInserted
    ));
    assert!(matches!(&events[3].event_type, SessionEventType::Done));
    assert!(matches!(
        &events[4].event_type,
        SessionEventType::TurnFinished
    ));
    assert!(matches!(
        &events[5].event_type,
        SessionEventType::UserMessage
    ));
    assert!(matches!(&events[8].event_type, SessionEventType::Done));
    assert!(matches!(
        &events[9].event_type,
        SessionEventType::TurnFinished
    ));
    assert_eq!(events[0].payload_json["delivery"], "immediate");
    assert_eq!(events[0].payload_json["attachments"], json!([]));
    assert_eq!(events[2].payload_json["turn_sequence"], 1);
    assert_eq!(
        events[3].payload_json["context_window"],
        json!({"total_tokens": 111})
    );
    assert_eq!(
        events[8].payload_json["context_window"],
        json!({"total_tokens": 222})
    );

    let tail_turn_id = messages[0].turn_id.expect("tail turn id");
    let session_turn = store
        .get_session_turn(session_id, tail_turn_id)
        .await
        .expect("load session turn")
        .expect("session turn");
    assert_eq!(session_turn.status, SessionTurnStatus::Completed);
    assert_eq!(session_turn.user_message_id, Some(messages[0].id));
    assert_eq!(session_turn.start_seq, Some(events[5].seq));
    // Store projection refresh canonicalizes persisted terminal bounds to
    // TurnFinished; materialize::tests covers construction with Done seq.
    assert_eq!(session_turn.end_seq, Some(events[9].seq));
    assert_eq!(
        session_turn.metrics_json,
        Some(json!({"total_tokens": 222}))
    );
}
