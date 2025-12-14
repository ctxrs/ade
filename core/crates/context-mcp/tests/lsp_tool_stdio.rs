use std::time::Duration;

use axum::{Json, Router, routing::post};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tower::ServiceBuilder;

#[tokio::test]
async fn mcp_lsp_diagnostics_calls_daemon_http() {
    let app = Router::new()
        .route(
            "/api/lsp/diagnostics",
            post(|| async {
                Json(json!([
                    {"message":"Intentional diagnostic from mock daemon"}
                ]))
            }),
        )
        // no auth for test server
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = env!("CARGO_BIN_EXE_context-mcp");
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CONTEXT_DAEMON_URL", format!("http://{}", addr))
        .env("CONTEXT_SESSION_ID", "00000000-0000-0000-0000-000000000000")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    stdin
        .write_all(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();

    stdin
        .write_all(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}).to_string().as_bytes())
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();

    stdin
        .write_all(
            json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"context.lsp_diagnostics",
                    "arguments":{"path":"src/lib.rs"}
                }
            })
            .to_string()
            .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let mut got_call = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else { break };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(3) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("Intentional diagnostic from mock daemon"));
            got_call = true;
            break;
        }
    }

    assert!(got_call, "did not receive tools/call response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_lsp_status_calls_daemon_http() {
    let app = Router::new()
        .route(
            "/api/lsp/status",
            axum::routing::get(|| async {
                Json(json!({"enabled":true,"edit_plans_enabled":true,"servers":[]}))
            }),
        )
        // no auth for test server
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = env!("CARGO_BIN_EXE_context-mcp");
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CONTEXT_DAEMON_URL", format!("http://{}", addr))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    stdin
        .write_all(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();

    stdin
        .write_all(
            json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{"name":"context.lsp_status","arguments":{}}
            })
            .to_string()
            .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let mut got_call = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else { break };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("\"enabled\": true"));
            got_call = true;
            break;
        }
    }
    assert!(got_call, "did not receive tools/call response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_lsp_rename_plan_and_apply_call_daemon_http() {
    let rename_body_tx = std::sync::Arc::new(tokio::sync::Mutex::new(None::<serde_json::Value>));
    let rename_body_tx2 = rename_body_tx.clone();

    let app = Router::new()
        .route(
            "/api/lsp/rename/plan",
            post(move |Json(body): Json<serde_json::Value>| {
                let rename_body_tx2 = rename_body_tx2.clone();
                async move {
                    *rename_body_tx2.lock().await = Some(body);
                    Json(json!({
                        "id":"pid-123",
                        "title":"Rename",
                        "created_at":"2025-01-01T00:00:00Z",
                        "remaining_files":1,
                        "remaining_hunks":1,
                        "diff":"diff --git a/a.txt b/a.txt\\n--- a/a.txt\\n+++ b/a.txt\\n@@ -1 +1 @@\\n-old\\n+new\\n"
                    }))
                }
            }),
        )
        .route(
            "/api/edit_plans/pid-123",
            axum::routing::get(|| async {
                Json(json!({
                    "id":"pid-123",
                    "title":"Rename",
                    "created_at":"2025-01-01T00:00:00Z",
                    "remaining_files":1,
                    "remaining_hunks":1,
                    "diff":"diff --git a/a.txt b/a.txt\\n--- a/a.txt\\n+++ b/a.txt\\n@@ -1 +1 @@\\n-old\\n+new\\n"
                }))
            }),
        )
        .route(
            "/api/edit_plans/pid-123/apply",
            post(|| async {
                Json(json!({
                    "id":"pid-123",
                    "title":"Rename",
                    "created_at":"2025-01-01T00:00:00Z",
                    "remaining_files":0,
                    "remaining_hunks":0,
                    "diff":""
                }))
            }),
        )
        // no auth for test server
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = env!("CARGO_BIN_EXE_context-mcp");
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CONTEXT_DAEMON_URL", format!("http://{}", addr))
        .env("CONTEXT_SESSION_ID", "00000000-0000-0000-0000-000000000000")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    stdin
        .write_all(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();

    // Create rename plan.
    stdin
        .write_all(
            json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"context.lsp_rename_plan",
                    "arguments":{"path":"src/lib.rs","line":0,"character":0,"new_name":"better"}
                }
            })
            .to_string()
            .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();

    // Apply entire edit plan (no patch arg triggers GET + apply).
    stdin
        .write_all(
            json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"context.apply_edit_plan",
                    "arguments":{"plan_id":"pid-123","action":"accept"}
                }
            })
            .to_string()
            .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_apply = false;
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else { break };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(3) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("\"remaining_hunks\": 0"));
            got_apply = true;
            break;
        }
    }
    assert!(got_apply, "did not receive tools/call apply response");

    let body = rename_body_tx.lock().await.clone().expect("expected rename body");
    assert_eq!(
        body.get("session_id").and_then(|v| v.as_str()),
        Some("00000000-0000-0000-0000-000000000000"),
        "expected session_id from env forwarded"
    );

    let _ = child.kill().await;
}
