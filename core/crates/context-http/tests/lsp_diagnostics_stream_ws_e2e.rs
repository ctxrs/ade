use std::collections::HashMap;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::json;

use context_http::api;
use context_http::daemon::AppState;
use context_lsp::LspManagerConfig;
use context_store::Store;

#[tokio::test]
async fn lsp_diagnostics_streams_over_global_ws_when_buffer_open() {
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let lsp_server = env!("CARGO_BIN_EXE_context-http-lsp-test-server").to_string();
    let state = std::sync::Arc::new(AppState::new_with_lsp_config_and_flags(
        data_dir.path().to_path_buf(),
        store.clone(),
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
        LspManagerConfig {
            enabled: true,
            rust_command: lsp_server,
            rust_args: vec![],
            diagnostics_wait: Duration::from_secs(2),
            ..Default::default()
        },
        false,
    ));

    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // Create a workspace+session quickly via HTTP.
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
    tokio::fs::create_dir_all(root.join("src")).await.unwrap();
    tokio::fs::write(root.join("src/lib.rs"), "pub fn ok() {}\n")
        .await
        .unwrap();
    tokio::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "ws"
version = "0.1.0"
edition = "2021"
"#,
    )
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
        .json(&json!({"provider_id":"fake","model_id":"fake"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Subscribe to global WS for this session id.
    let ws_url = format!("ws://{}/api/stream", addr);
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let sid_str = session.id.0.to_string();
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"type":"set","session_ids":[sid_str.clone()]}).to_string().into(),
        ))
        .await
        .unwrap();

    // Open buffer to trigger didOpen + publishDiagnostics and forwarding.
    let _open: serde_json::Value = client
        .post(format!("{base}/api/buffers/open"))
        .json(&json!({"session_id": session.id.0.to_string(), "path":"src/lib.rs"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Expect an lsp_diagnostics message.
    let mut got = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let Some(Ok(frame)) = ws_stream.next().await else { break };
        if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
            let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or(json!({}));
            if v.get("type").and_then(|t| t.as_str()) == Some("lsp_diagnostics") {
                assert_eq!(
                    v.get("session_id").and_then(|x| x.as_str()),
                    Some(sid_str.as_str())
                );
                assert_eq!(v.get("path").and_then(|x| x.as_str()), Some("src/lib.rs"));
                got = true;
                break;
            }
        }
    }
    assert!(got, "did not receive lsp_diagnostics message");

    server.abort();
}
