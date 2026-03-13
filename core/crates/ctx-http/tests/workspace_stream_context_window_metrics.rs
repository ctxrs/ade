use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

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

fn deltas_from_stream_message(value: &Value) -> Vec<Value> {
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "event" => {
            let Some(event) = value.get("event") else {
                return Vec::new();
            };
            if event.get("type").and_then(Value::as_str) != Some("session_head_delta") {
                return Vec::new();
            }
            event.get("delta").cloned().into_iter().collect()
        }
        "heads_batch" => value
            .get("deltas")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn assert_canonical_context_window(delta: &Value) {
    let turn = delta
        .get("turn")
        .and_then(Value::as_object)
        .expect("expected turn payload on terminal delta");
    let metrics = turn
        .get("metrics_json")
        .and_then(Value::as_object)
        .expect("expected canonical metrics_json on streamed turn");
    assert_eq!(
        metrics.get("context_window_tokens").and_then(Value::as_u64),
        Some(100)
    );
    assert_eq!(
        metrics
            .get("context_tokens_estimate")
            .and_then(Value::as_u64),
        Some(7)
    );
    assert_eq!(
        metrics
            .get("remaining_tokens_estimate")
            .and_then(Value::as_u64),
        Some(93)
    );
    let remaining_fraction = metrics
        .get("remaining_fraction")
        .and_then(Value::as_f64)
        .expect("expected remaining_fraction");
    assert!(
        (remaining_fraction - 0.93).abs() < 1e-9,
        "unexpected remaining_fraction: {remaining_fraction}"
    );
}

#[tokio::test]
async fn workspace_stream_done_delta_carries_context_window_metrics() {
    let (repo, _data_dir, _state, server) = setup().await;
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

    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title": "task"}))
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

    let ws_url = format!("{base}/api/workspaces/{}/stream", ws.id.0).replace("http://", "ws://");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();

    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "include_active_heads": true,
        "sessions": [{ "session_id": session.id.0, "after_seq": 0 }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let message: ctx_core::models::Message = client
        .post(format!("{base}/api/sessions/{}/messages", session.id.0))
        .json(&json!({"content":"slow-diff-test 0123456789"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let turn_id = message.turn_id.expect("expected turn id").0.to_string();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut saw_done = false;
    let mut saw_turn_finished = false;

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let next =
            tokio::time::timeout(remaining.min(Duration::from_millis(250)), socket.next()).await;
        match next {
            Ok(Some(Ok(WsMessage::Text(txt)))) => {
                let value: Value = serde_json::from_str(&txt).unwrap();
                for delta in deltas_from_stream_message(&value) {
                    let session_id = delta
                        .get("session_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if session_id != session.id.0.to_string() {
                        continue;
                    }
                    let Some(event) = delta.get("event").and_then(Value::as_object) else {
                        continue;
                    };
                    let event_turn_id = event
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if event_turn_id != turn_id {
                        continue;
                    }
                    match event
                        .get("event_type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                    {
                        "done" => {
                            assert_canonical_context_window(&delta);
                            saw_done = true;
                        }
                        "turn_finished" => {
                            assert_canonical_context_window(&delta);
                            saw_turn_finished = true;
                        }
                        _ => {}
                    }
                }
                if saw_done && saw_turn_finished {
                    break;
                }
            }
            Ok(Some(Ok(WsMessage::Close(_)))) => {
                panic!("workspace stream closed unexpectedly");
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(err))) => panic!("workspace stream error: {err:?}"),
            Ok(None) => panic!("workspace stream ended unexpectedly"),
            Err(_) => {}
        }
    }

    assert!(
        saw_done,
        "expected done delta with canonical context-window metrics"
    );
    assert!(
        saw_turn_finished,
        "expected turn_finished delta with canonical context-window metrics"
    );
}
