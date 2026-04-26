#![cfg(feature = "fault_injection")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use ctx_core::models::{
    Message, MessageRole, SessionEvent, SessionEventType, SessionTurn, SessionTurnStatus,
};
use ctx_http::daemon::AppState;
use serde_json::json;

mod common;

fn clear_all_failpoints() {
    ctx_http::fault_injection::clear_failpoints();
    ctx_store::fault_injection::clear_failpoints();
}

async fn post_message(app: &axum::Router, session_id: uuid::Uuid, content: &str) -> Message {
    let (status, msg): (StatusCode, Message) = common::json_request(
        app,
        Method::POST,
        format!("/api/sessions/{session_id}/messages"),
        Some(json!({"content": content})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    msg
}

async fn wait_for_terminal_turn(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    turn_id: ctx_core::ids::TurnId,
) -> (SessionTurn, Vec<SessionEvent>) {
    let store = state.store_for_session(session_id).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let turn = store
            .get_session_turn(session_id, turn_id)
            .await
            .unwrap()
            .expect("turn should exist");
        let events = store
            .list_session_events_for_turn(session_id, turn_id, false)
            .await
            .unwrap();
        let terminal = matches!(
            turn.status,
            SessionTurnStatus::Completed
                | SessionTurnStatus::Failed
                | SessionTurnStatus::Interrupted
        );
        let finished = events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::TurnFinished));
        if terminal && finished {
            return (turn, events);
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for terminal turn: {events:#?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn assistant_message_persistence_faults_recover_or_fail_honestly() {
    clear_all_failpoints();
    {
        let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
        let data_dir = tempfile::tempdir().unwrap();
        let stores = common::setup_store(data_dir.path()).await;
        let state = common::build_state(
            data_dir.path().to_path_buf(),
            stores,
            common::fake_providers(),
            "http://127.0.0.1:0",
        );
        let app = common::router(state.clone());

        ctx_http::fault_injection::set_failpoint("ctx_http.persist_assistant_message.transient", 1);

        let ws = common::create_workspace(&app, repo.path(), "ws").await;
        let (task, session) =
            common::create_task_with_session(&app, ws.id.0, "t1", "fake", "fake-model").await;
        let user_message = post_message(&app, session.id.0, "retry me").await;
        let turn_id = user_message.turn_id.expect("turn id");

        let (turn, events) = wait_for_terminal_turn(&state, session.id, turn_id).await;
        assert_eq!(turn.status, SessionTurnStatus::Completed);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event_type, SessionEventType::Error)),
            "transient persistence failure should recover cleanly: {events:#?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event_type,
                    SessionEventType::AssistantMessageInserted
                ))
                .count(),
            1,
            "expected exactly one inserted assistant message after retry: {events:#?}"
        );

        let store = state.store_for_session(session.id).await.unwrap();
        let assistant_messages = store
            .list_messages_for_session(session.id)
            .await
            .unwrap()
            .into_iter()
            .filter(|message| {
                message.turn_id == Some(turn_id) && matches!(message.role, MessageRole::Assistant)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            assistant_messages.len(),
            1,
            "assistant message should persist once"
        );
        assert_eq!(assistant_messages[0].content, "done: retry me");

        let turn_finished_statuses = events
            .iter()
            .filter(|event| matches!(event.event_type, SessionEventType::TurnFinished))
            .map(|event| {
                event
                    .payload_json
                    .get("status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("<missing>")
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(turn_finished_statuses, vec!["completed".to_string()]);

        clear_all_failpoints();
    }

    {
        let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
        let data_dir = tempfile::tempdir().unwrap();
        let stores = common::setup_store(data_dir.path()).await;
        let state = common::build_state(
            data_dir.path().to_path_buf(),
            stores,
            common::fake_providers(),
            "http://127.0.0.1:0",
        );
        let app = common::router(state.clone());

        let ws = common::create_workspace(&app, repo.path(), "ws").await;
        let (task, session) =
            common::create_task_with_session(&app, ws.id.0, "t1", "fake", "fake-model").await;
        let user_message = post_message(&app, session.id.0, "retry after partial write").await;
        ctx_store::fault_injection::set_failpoint("ctx_store.insert_message.after_insert", 1);
        let turn_id = user_message.turn_id.expect("turn id");

        let (turn, events) = wait_for_terminal_turn(&state, session.id, turn_id).await;
        assert_eq!(turn.status, SessionTurnStatus::Completed);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event_type, SessionEventType::Error)),
            "post-insert transient persistence failure should recover cleanly: {events:#?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event_type,
                    SessionEventType::AssistantMessageInserted
                ))
                .count(),
            1,
            "expected exactly one inserted assistant message after transactional retry: {events:#?}"
        );

        let store = state.store_for_session(session.id).await.unwrap();
        let assistant_messages = store
            .list_messages_for_session(session.id)
            .await
            .unwrap()
            .into_iter()
            .filter(|message| {
                message.turn_id == Some(turn_id) && matches!(message.role, MessageRole::Assistant)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            assistant_messages.len(),
            1,
            "assistant message should not be duplicated after post-insert retry"
        );
        assert_eq!(
            assistant_messages[0].content,
            "done: retry after partial write"
        );

        clear_all_failpoints();
    }

    {
        let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
        let data_dir = tempfile::tempdir().unwrap();
        let stores = common::setup_store(data_dir.path()).await;
        let state = common::build_state(
            data_dir.path().to_path_buf(),
            stores,
            common::fake_providers(),
            "http://127.0.0.1:0",
        );
        let app = common::router(state.clone());

        ctx_http::fault_injection::set_failpoint("ctx_http.persist_assistant_message.fatal", 1);

        let ws = common::create_workspace(&app, repo.path(), "ws").await;
        let (task, session) =
            common::create_task_with_session(&app, ws.id.0, "t1", "fake", "fake-model").await;
        let user_message = post_message(&app, session.id.0, "fail me").await;
        let turn_id = user_message.turn_id.expect("turn id");

        let (turn, events) = wait_for_terminal_turn(&state, session.id, turn_id).await;
        assert_eq!(turn.status, SessionTurnStatus::Failed);
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event_type, SessionEventType::Error)),
            "fatal assistant persistence failure must surface as an Error event: {events:#?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event_type,
                    SessionEventType::AssistantMessageInserted
                ))
                .count(),
            0,
            "assistant message insert should fail in this scenario: {events:#?}"
        );

        let turn_finished_statuses = events
            .iter()
            .filter(|event| matches!(event.event_type, SessionEventType::TurnFinished))
            .map(|event| {
                event
                    .payload_json
                    .get("status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("<missing>")
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            turn_finished_statuses,
            vec!["failed".to_string()],
            "fatal assistant persistence failure must not emit a completed TurnFinished event: {events:#?}"
        );

        let store = state.store_for_session(session.id).await.unwrap();
        let assistant_messages = store
            .list_messages_for_session(session.id)
            .await
            .unwrap()
            .into_iter()
            .filter(|message| {
                message.turn_id == Some(turn_id) && matches!(message.role, MessageRole::Assistant)
            })
            .collect::<Vec<_>>();
        assert!(
            assistant_messages.is_empty(),
            "no assistant message should be persisted when the fatal failpoint is armed"
        );

        clear_all_failpoints();
    }
}
