use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;

use context_http::api;
use context_http::daemon::AppState;
use context_providers::fake::FakeProviderAdapter;
use context_store::Store;

#[tokio::test]
async fn global_stream_replays_from_db_and_resumes_by_seq() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["init"])
        .output()
        .await
        .unwrap();
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.email", "test@example.com"])
        .output()
        .await
        .unwrap();
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name", "Test"])
        .output()
        .await
        .unwrap();
    tokio::fs::write(root.join("file.txt"), "hello\n")
        .await
        .unwrap();
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["add", "."])
        .output()
        .await
        .unwrap();
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["commit", "-m", "init"])
        .output()
        .await
        .unwrap();

    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn context_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        store.clone(),
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    state.start_workspace_index_listener();
    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    let ws: context_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({"root_path": root.to_string_lossy(), "name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let task: context_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"t1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tracks: Vec<context_core::models::Track> = client
        .get(format!("{base}/api/tasks/{}/tracks", task.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let track = &tracks[0];
    let session: context_core::models::Session = client
        .post(format!("{base}/api/tracks/{}/sessions", track.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Run once to populate the DB with events.
    let _msg: context_core::models::Message = client
        .post(format!("{base}/api/sessions/{}/messages", session.id.0))
        .json(&json!({"content":"hello"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Wait for Done to be persisted.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let evs: Vec<context_core::models::SessionEvent> = client
            .get(format!(
                "{base}/api/sessions/{}/events?tail=50",
                session.id.0
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if evs
            .iter()
            .any(|e| matches!(e.event_type, context_core::models::SessionEventType::Done))
        {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for Done");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Connect to global stream and request replay from seq 0.
    let ws_url = format!("ws://{}/api/stream", addr);
    let (mut stream, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let sid_str = session.id.0.to_string();
    stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"type":"set","sessions":[{"session_id":sid_str,"after_seq":0}]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let mut last_seq = 0i64;
    let mut saw_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let Some(Ok(frame)) = stream.next().await else {
            break;
        };
        if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
            let ev: context_core::models::SessionEvent = serde_json::from_str(&txt).unwrap();
            last_seq = last_seq.max(ev.seq);
            if matches!(ev.event_type, context_core::models::SessionEventType::Done) {
                saw_done = true;
                break;
            }
        }
    }
    assert!(saw_done, "did not replay Done over global stream");
    assert!(last_seq > 0);

    // Reconnect and resume from last_seq; should not replay anything quickly.
    let ws_url = format!("ws://{}/api/stream", addr);
    let (mut stream2, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    stream2
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"type":"set","sessions":[{"session_id":session.id.0.to_string(),"after_seq":last_seq}]}).to_string().into(),
        ))
        .await
        .unwrap();

    let no_frame = tokio::time::timeout(Duration::from_millis(250), stream2.next()).await;
    assert!(
        no_frame.is_err(),
        "unexpected replay when resuming from last_seq"
    );

    server.abort();
}
