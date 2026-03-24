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

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
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

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        let saw_interrupted = events.iter().any(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnInterrupted)
        });
        let saw_finished = events.iter().any(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnFinished)
        });
        let saw_queue_lifecycle = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::InputQueued)
        }) && events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::MessageQueueAdded)
        }) && events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::TurnQueued)
        });
        if saw_interrupted && saw_finished && saw_queue_lifecycle {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for interrupt + queue lifecycle events");
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

#[tokio::test]
async fn cancel_promotes_next_queued_turn_after_interrupted_finish() {
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
            panic!("timed out waiting for turn start + queue events");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/sessions/{}/cancel", session.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::OK);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        let saw_finished = events.iter().any(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnFinished)
        });
        let saw_promoted = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::MessageQueuePromoted)
        });
        let saw_next_started = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::TurnStarted)
        });
        if saw_finished && saw_promoted && saw_next_started {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for cancel promotion");
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

    let interrupted_seq = seq_for(turn_id_one, SessionEventType::TurnInterrupted);
    let finished_seq = seq_for(turn_id_one, SessionEventType::TurnFinished);
    let promoted_seq = seq_for(turn_id_two, SessionEventType::MessageQueuePromoted);
    let started_seq = seq_for(turn_id_two, SessionEventType::TurnStarted);

    assert!(interrupted_seq < finished_seq);
    assert!(interrupted_seq < promoted_seq);
    assert!(promoted_seq < started_seq);
}

#[tokio::test]
async fn cancel_promotes_queued_turns_in_fifo_order_across_multiple_cancels() {
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
        Some(json!({"content":"second slow-diff-test","delivery":"queued"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, msg3): (StatusCode, ctx_core::models::Message) = common::json_request(
        &app,
        Method::POST,
        format!("/api/sessions/{}/messages", session.id.0),
        Some(json!({"content":"third","delivery":"queued"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let turn_id_one = msg1.turn_id.expect("first turn id");
    let turn_id_two = msg2.turn_id.expect("second turn id");
    let turn_id_three = msg3.turn_id.expect("third turn id");
    let store = state.store_for_session(session.id).await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        let saw_started = events.iter().any(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnStarted)
        });
        let saw_second_queued = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::TurnQueued)
        });
        let saw_third_queued = events.iter().any(|event| {
            event.turn_id == Some(turn_id_three)
                && matches!(event.event_type, SessionEventType::TurnQueued)
        });
        if saw_started && saw_second_queued && saw_third_queued {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for queue chain");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let events = store.list_session_events(session.id).await.unwrap();
    let payload_i64_for = |turn_id, event_type, key| {
        let target = std::mem::discriminant(&event_type);
        events
            .iter()
            .find(|event| {
                event.turn_id == Some(turn_id)
                    && std::mem::discriminant(&event.event_type) == target
            })
            .and_then(|event| event.payload_json.get(key))
            .and_then(|value| value.as_i64())
    };
    assert_eq!(
        payload_i64_for(
            turn_id_two,
            SessionEventType::MessageQueueAdded,
            "queue_position"
        ),
        Some(0)
    );
    assert_eq!(
        payload_i64_for(
            turn_id_three,
            SessionEventType::MessageQueueAdded,
            "queue_position"
        ),
        Some(1)
    );
    assert_eq!(
        payload_i64_for(turn_id_two, SessionEventType::TurnQueued, "queue_position"),
        Some(0)
    );
    assert_eq!(
        payload_i64_for(
            turn_id_three,
            SessionEventType::TurnQueued,
            "queue_position"
        ),
        Some(1)
    );

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/sessions/{}/cancel", session.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::OK);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        let first_finished = events.iter().any(|event| {
            event.turn_id == Some(turn_id_one)
                && matches!(event.event_type, SessionEventType::TurnFinished)
        });
        let second_promoted = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::MessageQueuePromoted)
        });
        let second_started = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::TurnStarted)
        });
        if first_finished && second_promoted && second_started {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for first cancel promotion");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/sessions/{}/cancel", session.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, _) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::OK);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        let second_finished = events.iter().any(|event| {
            event.turn_id == Some(turn_id_two)
                && matches!(event.event_type, SessionEventType::TurnFinished)
        });
        let third_promoted = events.iter().any(|event| {
            event.turn_id == Some(turn_id_three)
                && matches!(event.event_type, SessionEventType::MessageQueuePromoted)
        });
        let third_started = events.iter().any(|event| {
            event.turn_id == Some(turn_id_three)
                && matches!(event.event_type, SessionEventType::TurnStarted)
        });
        if second_finished && third_promoted && third_started {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for second cancel promotion");
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
    let payload_i64_for = |turn_id, event_type, key| {
        let target = std::mem::discriminant(&event_type);
        events
            .iter()
            .find(|event| {
                event.turn_id == Some(turn_id)
                    && std::mem::discriminant(&event.event_type) == target
            })
            .and_then(|event| event.payload_json.get(key))
            .and_then(|value| value.as_i64())
    };

    let first_interrupted_seq = seq_for(turn_id_one, SessionEventType::TurnInterrupted);
    let first_finished_seq = seq_for(turn_id_one, SessionEventType::TurnFinished);
    let second_promoted_seq = seq_for(turn_id_two, SessionEventType::MessageQueuePromoted);
    let second_started_seq = seq_for(turn_id_two, SessionEventType::TurnStarted);
    let second_interrupted_seq = seq_for(turn_id_two, SessionEventType::TurnInterrupted);
    let second_finished_seq = seq_for(turn_id_two, SessionEventType::TurnFinished);
    let third_promoted_seq = seq_for(turn_id_three, SessionEventType::MessageQueuePromoted);
    let third_started_seq = seq_for(turn_id_three, SessionEventType::TurnStarted);

    assert!(first_interrupted_seq < first_finished_seq);
    assert!(first_interrupted_seq < second_promoted_seq);
    assert!(second_promoted_seq < second_started_seq);

    assert!(second_interrupted_seq < second_finished_seq);
    assert!(second_interrupted_seq < third_promoted_seq);
    assert!(third_promoted_seq < third_started_seq);
    assert!(second_promoted_seq < third_promoted_seq);

    let promoted_turns: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.event_type, SessionEventType::MessageQueuePromoted))
        .map(|event| event.turn_id)
        .collect();
    assert_eq!(promoted_turns, vec![Some(turn_id_two), Some(turn_id_three)]);
    assert_eq!(
        payload_i64_for(
            turn_id_two,
            SessionEventType::MessageQueuePromoted,
            "previous_position"
        ),
        Some(0)
    );
    assert_eq!(
        payload_i64_for(
            turn_id_three,
            SessionEventType::MessageQueuePromoted,
            "previous_position",
        ),
        Some(0)
    );
}
