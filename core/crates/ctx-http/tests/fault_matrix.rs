#![cfg(feature = "fault_injection")]

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

async fn setup_server() -> (
    Arc<AppState>,
    Store,
    tokio::task::JoinHandle<()>,
    std::net::SocketAddr,
    ctx_core::models::Workspace,
    ctx_core::models::Session,
    i64,
) {
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
        .json(&json!({"title":"fault-matrix"}))
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

    let last = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            json!({"msg":"last"}),
        )
        .await
        .unwrap()
        .seq;

    (state, store, server, addr, ws, session, last)
}

#[tokio::test]
async fn fault_matrix_replay_errors_become_gaps() {
    let (_state, store, server, addr, ws, session, last_seq) = setup_server().await;

    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    struct Case {
        name: &'static str,
        setup: fn(),
    }

    let cases = [
        Case {
            name: "store list fails",
            setup: || {
                ctx_http::fault_injection::clear_failpoints();
                ctx_store::fault_injection::clear_failpoints();
                ctx_store::fault_injection::set_failpoint(
                    "ctx_store.list_session_events_page_by_seq",
                    1,
                );
            },
        },
        Case {
            name: "http replay list fails",
            setup: || {
                ctx_http::fault_injection::clear_failpoints();
                ctx_store::fault_injection::clear_failpoints();
                ctx_http::fault_injection::set_failpoint("ctx_http.replay_session_events.list", 1);
            },
        },
    ];

    for case in cases {
        (case.setup)();
        let subscribe = json!({
            "type": "subscribe",
            "sessions": [{
                "session_id": session.id.0,
                "after_seq": 0,
            }],
        })
        .to_string();
        socket
            .send(WsMessage::Text(subscribe.into()))
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        let WsMessage::Text(txt) = msg else {
            panic!("{}: expected text frame, got {:?}", case.name, msg);
        };
        let event: WorkspaceActiveSnapshotEvent = serde_json::from_str(&txt).unwrap();
        match event {
            WorkspaceActiveSnapshotEvent::SessionGap {
                session_id,
                after_seq,
                reason,
                ..
            } => {
                assert_eq!(session_id, session.id, "{}", case.name);
                assert_eq!(after_seq, last_seq, "{}", case.name);
                assert_eq!(reason.as_deref(), Some("replay_error"), "{}", case.name);
            }
            other => panic!("{}: expected SessionGap, got {:?}", case.name, other),
        }

        ctx_http::fault_injection::clear_failpoints();
        ctx_store::fault_injection::clear_failpoints();
    }

    server.abort();
    drop(store);
}
