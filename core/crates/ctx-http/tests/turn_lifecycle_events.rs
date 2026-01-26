use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::json;

use ctx_core::models::SessionEventType;

mod common;

#[tokio::test]
async fn queued_message_emits_lifecycle_events_in_order_with_interrupt() {
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
    let task = common::create_task(&app, ws.id.0, "t1").await;
    let session = common::create_session(&app, task.id.0, "fake", "fake-model").await;

    let (status, msg1): (StatusCode, ctx_core::models::Message) = common::json_request(
        &app,
        Method::POST,
        format!("/api/sessions/{}/messages", session.id.0),
        Some(json!({"content":"first slow-diff-test"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, msg2): (StatusCode, ctx_core::models::Message) = common::json_request(
        &app,
        Method::POST,
        format!("/api/sessions/{}/messages", session.id.0),
        Some(json!({"content":"second","delivery":"queued"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let turn_id_one = msg1.turn_id.expect("first turn id");
    let turn_id_two = msg2.turn_id.expect("second turn id");
    let store = state.store_for_session(session.id).await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        let saw_started = events.iter().any(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnStarted)
        });
        let saw_queued = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::TurnQueued)
        });
        if saw_started && saw_queued {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for lifecycle events");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/sessions/{}/interrupt", session.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::OK);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        let saw_interrupted = events.iter().any(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnInterrupted)
        });
        if saw_interrupted {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for interrupt event");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let events = store.list_session_events(session.id).await.unwrap();
    let seq_for = |turn_id, event_type| {
        let target = std::mem::discriminant(&event_type);
        events
            .iter()
            .find(|event| {
                event.turn_id == Some(turn_id)
                    && std::mem::discriminant(&event.event_type) == target
            })
            .map(|event| event.seq)
            .expect("expected event seq")
    };

    let user_seq = seq_for(turn_id_two, SessionEventType::UserMessage);
    let input_seq = seq_for(turn_id_two, SessionEventType::InputQueued);
    let queue_seq = seq_for(turn_id_two, SessionEventType::MessageQueueAdded);
    let turn_seq = seq_for(turn_id_two, SessionEventType::TurnQueued);
    assert!(user_seq < input_seq && input_seq < queue_seq && queue_seq < turn_seq);

    let started_seq = seq_for(turn_id_one, SessionEventType::TurnStarted);
    let interrupted_seq = seq_for(turn_id_one, SessionEventType::TurnInterrupted);
    let finished_seq = seq_for(turn_id_one, SessionEventType::TurnFinished);
    assert!(started_seq < interrupted_seq && interrupted_seq < finished_seq);

    let finished = events
        .iter()
        .find(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnFinished)
        })
        .expect("expected finished event");
    assert_eq!(
        finished
            .payload_json
            .get("status")
            .and_then(|value| value.as_str()),
        Some("interrupted")
    );
}
