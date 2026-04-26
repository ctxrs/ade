use std::time::Duration;

use axum::http::{Method, StatusCode};
use serde_json::json;

use ctx_core::models::SessionEventType;

mod common;

#[tokio::test]
async fn assistant_chunks_are_stream_only() {
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
    let (_task, session) =
        common::create_task_with_session(&app, ws.id.0, "t1", "fake", "fake-model").await;

    let (status, _msg): (StatusCode, ctx_core::models::Message) = common::json_request(
        &app,
        Method::POST,
        format!("/api/sessions/{}/messages", session.id.0),
        Some(json!({"content":"hello"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let store = state.store_for_session(session.id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        if events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::Done))
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for Done event");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let events = store.list_session_events(session.id).await.unwrap();
    assert!(events
        .iter()
        .all(|event| !matches!(event.event_type, SessionEventType::AssistantChunk)));

    let turns = store
        .list_session_turns_page_by_seq(session.id, None, Some(1))
        .await
        .unwrap();
    assert!(!turns.is_empty());
    assert!(turns.iter().all(|turn| turn.assistant_partial.is_none()));
}
