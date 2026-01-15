#![cfg(feature = "property_tests")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::process::Command;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_core::models::{SessionEventType, WorkspaceActiveSnapshotEvent};
use ctx_http::{api, daemon::AppState};
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::Store;

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["init"])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.email", "test@example.com"])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name", "Test"])
        .output()
        .await
        .unwrap();
    tokio::fs::write(root.join("file.txt"), "hello\n")
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["add", "."])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["commit", "-m", "init"])
        .output()
        .await
        .unwrap();
    dir
}

#[tokio::test]
async fn property_replay_respects_after_seq_and_monotonicity() {
    let repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        store.clone(),
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    state.start_workspace_active_snapshot_listener();
    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();

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
        .json(&json!({"title":"replay-props"}))
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
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert!(
        sessions.iter().any(|stored| stored.id == session.id),
        "expected session to be stored"
    );

    let mut seqs = Vec::new();
    for i in 0..15 {
        let ev = store
            .append_session_event(
                session.id,
                None,
                None,
                SessionEventType::Notice,
                json!({"i":i}),
            )
            .await
            .unwrap();
        seqs.push(ev.seq);
    }
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));

    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Bounded, deterministic set of after_seq values covering edges + middle.
    let after_seqs = [0, seqs[0], seqs[3], seqs[7], seqs[14], seqs[14] + 10];

    for after_seq in after_seqs {
        let expected: Vec<i64> = seqs.iter().copied().filter(|s| *s > after_seq).collect();
        let subscribe = json!({
            "type": "subscribe",
            "sessions": [{
                "session_id": session.id.0,
                "after_seq": after_seq,
            }],
        })
        .to_string();
        socket
            .send(WsMessage::Text(subscribe.into()))
            .await
            .unwrap();

        let mut got = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while got.len() < expected.len() && tokio::time::Instant::now() < deadline {
            let Some(Ok(WsMessage::Text(txt))) = socket.next().await else {
                break;
            };
            let Ok(WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. }) =
                serde_json::from_str::<WorkspaceActiveSnapshotEvent>(&txt)
            else {
                continue;
            };
            if delta.session_id != session.id {
                continue;
            }
            let Some(event) = delta.event else {
                continue;
            };
            assert_eq!(delta.last_event_seq, event.seq);
            got.push(event.seq);
        }

        assert_eq!(got, expected, "after_seq={after_seq}");
        assert!(got.windows(2).all(|w| w[0] < w[1]), "after_seq={after_seq}");
    }

    server.abort();
    drop(state);
}
