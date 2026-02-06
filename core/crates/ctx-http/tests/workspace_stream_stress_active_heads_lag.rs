use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::time::Duration;

use chrono::Utc;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use ctx_core::ids::{MessageId, SessionEventId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, SessionEvent, SessionEventType, SessionHeadDelta,
    SessionTurn, SessionTurnStatus,
};
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

fn big_content(bytes: usize) -> String {
    // Deterministic and compresses poorly enough to stay large over JSON.
    // (Some repeated patterns compress well in OS stacks, but JSON encoding is still big.)
    let mut out = String::with_capacity(bytes);
    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789";
    while out.len() < bytes {
        let idx = out.len() % alphabet.len();
        out.push(alphabet[idx] as char);
    }
    out
}

#[tokio::test]
#[ignore]
async fn workspace_stream_does_not_reset_during_hydration_when_active_heads_are_large() {
    // This is a stress/soak-style regression test.
    // Expected behavior:
    // - With compact active heads (fixed code), the initial snapshot should be small enough that
    //   hydration completes quickly and buffered head deltas do not overflow.
    // - On older baselines (without compaction), hydration can take long enough that the head
    //   batch buffer overflows, causing reset_required.

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

    // Create many sessions to inflate active_heads total payload.
    // Keep the snapshot under tungstenite's default max message size (16 MiB) so
    // baselines fail (if they fail) due to stream backpressure/reset behavior,
    // not client-side frame limits.
    let session_count: usize = 12;
    let turns_per_session: i64 = 70;
    let message_bytes: usize = 16 * 1024;

    let mut sessions: Vec<ctx_core::models::Session> = Vec::with_capacity(session_count);
    for i in 0..session_count {
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

    // Hydrate active snapshot so update_session_head knows which sessions are primary.
    state.ensure_workspace_active_snapshot_hydrated(ws.id).await;

    let content = big_content(message_bytes);

    // Seed turns+messages directly in the store to create large session heads.
    for session in &sessions {
        let store = state.store_for_session(session.id).await.unwrap();
        for t in 0..turns_per_session {
            let turn_id = TurnId::new();
            let at = common::fixed_utc(t);
            let turn = SessionTurn {
                turn_id,
                session_id: session.id,
                run_id: None,
                user_message_id: None,
                status: SessionTurnStatus::Completed,
                start_seq: Some(t),
                end_seq: Some(t),
                started_at: at,
                updated_at: at,
                assistant_partial: None,
                thought_partial: None,
                metrics_json: None,
                tool_total: 0,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 0,
                tool_failed: 0,
            };
            store.insert_session_turn(turn).await.unwrap();

            let msg = Message {
                id: MessageId::new(),
                session_id: session.id,
                task_id: session.task_id,
                run_id: None,
                turn_id: Some(turn_id),
                turn_sequence: Some(t),
                order_seq: None,
                role: MessageRole::User,
                content: content.clone(),
                attachments: Vec::new(),
                delivery: MessageDelivery::Immediate,
                delivered_at: None,
                created_at: at,
            };
            store.insert_message(msg).await.unwrap();
        }

        // Cache the head into workspace active snapshot.
        let head = store
            .get_session_head_snapshot(session.id, 200, true)
            .await
            .unwrap()
            .unwrap();
        state
            .workspaces
            .workspace_active_snapshot
            .update_session_head(head)
            .await;
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
            .map(|s| json!({"session_id": s.id.0, "after_seq": 0}))
            .collect::<Vec<_>>(),
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    // Publish head deltas until we observe the snapshot. The stream does not flush heads batches
    // while hydrating, so on a slow/large snapshot this should eventually overflow and reset.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_pub = stop.clone();
    let state_pub = state.clone();
    let ws_id = ws.id;
    let sessions_pub = sessions.clone();

    let publisher = tokio::spawn(async move {
        let mut seq: i64 = 1;
        let mut idx: usize = 0;
        let mut throttle: u32 = 0;
        while !stop_pub.load(Ordering::Relaxed) {
            let session = &sessions_pub[idx % sessions_pub.len()];
            idx += 1;

            let delta = SessionHeadDelta {
                session_id: session.id,
                last_event_seq: seq,
                state_rev: 0,
                event: Some(SessionEvent {
                    seq,
                    id: SessionEventId::new(),
                    session_id: session.id,
                    run_id: None,
                    turn_id: None,
                    event_type: SessionEventType::Done,
                    payload_json: json!({"ok": true}),
                    transient: false,
                    created_at: Utc::now(),
                }),
                turn: None,
                message: None,
                tool_summaries: Vec::new(),
            };
            state_pub
                .workspaces
                .workspace_active_snapshot
                .publish_session_head_delta(ws_id, session, delta, false)
                .await;
            seq += 1;

            // Throttle just enough for the sender task to progress on small snapshots.
            // On baselines with large active_heads, hydration should still take long enough to
            // overflow the head batch buffer.
            throttle = throttle.wrapping_add(1);
            if throttle % 8 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });

    // Wait for the snapshot and fail fast if the server resets while hydrating.
    let mut saw_snapshot = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        let next = tokio::time::timeout(Duration::from_millis(250), socket.next()).await;
        match next {
            Ok(Some(Ok(WsMessage::Text(txt)))) => {
                let value: Value = serde_json::from_str(&txt).unwrap();
                let msg_type = value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match msg_type {
                    "reset_required" => {
                        panic!("unexpected reset_required while hydrating with active_heads");
                    }
                    "snapshot" => {
                        saw_snapshot = true;
                        break;
                    }
                    _ => {}
                }
            }
            Ok(Some(Ok(WsMessage::Close(_)))) => {
                panic!("workspace stream closed before snapshot");
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(err))) => panic!("workspace stream error: {err:?}"),
            Ok(None) => panic!("workspace stream ended before snapshot"),
            Err(_) => {}
        }
    }

    if !saw_snapshot {
        panic!("timed out waiting for snapshot");
    }

    stop.store(true, Ordering::Relaxed);
    publisher.await.unwrap();

    // Post-snapshot, we should remain live without reset_required.
    let post_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < post_deadline {
        let next = tokio::time::timeout(Duration::from_millis(200), socket.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(txt)))) = next {
            let value: Value = serde_json::from_str(&txt).unwrap();
            let msg_type = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if msg_type == "reset_required" {
                panic!("unexpected reset_required after snapshot");
            }
        }
    }
}
