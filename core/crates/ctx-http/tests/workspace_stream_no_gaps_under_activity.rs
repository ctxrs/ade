use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use ctx_http::daemon::AppState;

mod common;

async fn setup() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
    common::TestServer,
) {
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
    let server = common::spawn_http_server(app).await;

    (repo, data_dir, state, server)
}

async fn sessions_have_done_events_in_store(
    state: &Arc<AppState>,
    sessions: &[ctx_core::models::Session],
    expected_done_events_per_session: usize,
) -> bool {
    for session in sessions {
        let store = state.store_for_session(session.id).await.unwrap();
        let events = store.list_session_events(session.id).await.unwrap();
        if events
            .iter()
            .any(|event| matches!(event.event_type, ctx_core::models::SessionEventType::Error))
        {
            panic!("unexpected session error while running activity: {events:#?}");
        }
        let done_count = events
            .iter()
            .filter(|event| matches!(event.event_type, ctx_core::models::SessionEventType::Done))
            .count();
        if done_count < expected_done_events_per_session {
            return false;
        }
    }
    true
}

#[tokio::test]
async fn workspace_stream_stays_live_without_gaps_under_activity() {
    // This test is intentionally moderate: it should be stable in CI but still
    // exercise streaming with tool calls + thought chunks across multiple sessions.
    let (repo, _data_dir, state, server) = setup().await;
    let base = &server.base_url;
    let client = &server.client;

    let ws: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": repo.path(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut sessions: Vec<ctx_core::models::Session> = Vec::new();
    for i in 0..3 {
        let task: ctx_core::models::Task = client
            .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
            .json(&json!({"title": format!("task-{i}")}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        let session: ctx_core::models::Session = client
            .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
            .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        sessions.push(session);
    }

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();

    // Drain the initial server frame (ready).
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
        "sessions": sessions
            .iter()
            .map(|s| {
                json!({
                    "session_id": s.id.0,
                    "replay": {
                        "mode": "resume",
                        "after_seq": 0,
                    },
                })
            })
            .collect::<Vec<_>>(),
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    // Fire activity: multiple turns per session. Fake provider emits assistant chunk,
    // thought chunks (opt-in marker), tool call/result, assistant complete, done.
    let mut senders = Vec::new();
    for (idx, session) in sessions.iter().enumerate() {
        let client = client.clone();
        let base = base.to_string();
        let session_id = session.id.0;
        senders.push(tokio::spawn(async move {
            for j in 0..5 {
                let content = format!("turn {idx}/{j} emit-thought");
                let _resp: ctx_core::models::Message = client
                    .post(format!("{base}/api/sessions/{session_id}/messages"))
                    .json(&json!({"content": content}))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
            }
        }));
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for activity without gaps/reset_required");
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        match next {
            Ok(Some(Ok(WsMessage::Text(txt)))) => {
                let value: Value = serde_json::from_str(&txt).unwrap();
                let msg_type = value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match msg_type {
                    "reset_required" => {
                        panic!("unexpected reset_required while running activity");
                    }
                    "event" => {
                        let Some(event) = value.get("event") else {
                            continue;
                        };
                        let event_type = event
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if event_type == "session_gap" {
                            panic!("unexpected session_gap while running activity");
                        }

                        if event_type == "session_head_delta" {}
                    }
                    "heads_batch" => {}
                    "snapshot" => {}
                    _ => {}
                }
            }
            Ok(Some(Ok(WsMessage::Close(_)))) => {
                panic!("workspace stream closed unexpectedly while running activity");
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(err))) => panic!("workspace stream error: {err:?}"),
            Ok(None) => panic!("workspace stream ended unexpectedly"),
            Err(_) => {
                // no frame in this interval; check completion progress
            }
        }

        let all_sent = senders.iter().all(|h| h.is_finished());
        let enough_done = if all_sent {
            sessions_have_done_events_in_store(&state, &sessions, 5).await
        } else {
            false
        };
        if all_sent && enough_done {
            break;
        }
    }

    for h in senders {
        h.await.unwrap();
    }
}
