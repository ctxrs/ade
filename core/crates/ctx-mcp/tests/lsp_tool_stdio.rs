use std::time::Duration;

use axum::{routing::post, Json, Router};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tower::ServiceBuilder;

fn mcp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ctx-mcp")
}

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

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
        .env("CTX_SESSION_ID", "00000000-0000-0000-0000-000000000000")
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
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
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
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"ctx.lsp_diagnostics",
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
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
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
#[ignore = "Web sessions temporarily disabled; re-enable when MCP web session tools return."]
async fn mcp_web_session_tools_call_daemon_http() {
    let app = Router::new()
        .route(
            "/api/sessions/web",
            post(|Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["url"], "https://example.com");
                Json(json!({
                    "id": "sess-1",
                    "kind": "web",
                    "status": "running",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "last_activity": "2026-01-01T00:00:00Z",
                    "url": "https://example.com",
                    "viewport": {"width": 1280, "height": 720},
                    "fps": 30,
                    "viewers": 0,
                    "stream_path": "/sessions/web/sess-1/view",
                    "stream_url": "http://127.0.0.1:0/sessions/web/sess-1/view"
                }))
            }),
        )
        .route(
            "/api/sessions/web/sess-1/eval",
            post(|| async {
                Json(json!({
                    "ok": true,
                    "result": "Example Domain",
                    "error": null
                }))
            }),
        )
        .route(
            "/api/sessions/web/sess-1/close",
            post(|| async { axum::http::StatusCode::NO_CONTENT }),
        )
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    for msg in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                "name":"session_create",
                "arguments":{
                    "kind":"web",
                    "target":{"url":"https://example.com"}
                }
            }
        }),
        json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                "name":"session_eval",
                "arguments":{
                    "kind":"web",
                    "session_id":"sess-1",
                    "code":"return await page.title()"
                }
            }
        }),
        json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                "name":"session_close",
                "arguments":{
                    "kind":"web",
                    "session_id":"sess-1"
                }
            }
        }),
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let mut got_create = false;
    let mut got_eval = false;
    let mut got_close = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        match v.get("id").and_then(|id| id.as_i64()) {
            Some(3) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                let payload: serde_json::Value = serde_json::from_str(text).unwrap();
                assert_eq!(payload["id"], "sess-1");
                got_create = true;
            }
            Some(4) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                let payload: serde_json::Value = serde_json::from_str(text).unwrap();
                assert_eq!(payload["ok"], true);
                assert_eq!(payload["result"], "Example Domain");
                got_eval = true;
            }
            Some(5) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                let payload: serde_json::Value = serde_json::from_str(text).unwrap();
                assert_eq!(payload["closed"], true);
                got_close = true;
            }
            _ => {}
        }
        if got_create && got_eval && got_close {
            break;
        }
    }

    assert!(got_create, "did not receive session_create response");
    assert!(got_eval, "did not receive session_eval response");
    assert!(got_close, "did not receive session_close response");

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

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
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
                "params":{"name":"ctx.lsp_status","arguments":{}}
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
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
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
async fn mcp_lsp_hover_calls_daemon_http() {
    let app = Router::new()
        .route(
            "/api/lsp/hover",
            post(|| async { Json(json!({"contents":{"kind":"markdown","value":"hover ok"}})) }),
        )
        // no auth for test server
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
        .env("CTX_SESSION_ID", "00000000-0000-0000-0000-000000000000")
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
                "params":{
                    "name":"ctx.lsp_hover",
                    "arguments":{"path":"src/lib.rs","line":0,"character":0}
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
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("hover ok"));
            got_call = true;
            break;
        }
    }
    assert!(got_call, "did not receive tools/call response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_lsp_execute_command_calls_daemon_http() {
    let app = Router::new()
        .route(
            "/api/lsp/execute_command",
            post(|| async { Json(json!({"result":{"ok":true},"workspace_edit":{}})) }),
        )
        // no auth for test server
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
        .env("CTX_SESSION_ID", "00000000-0000-0000-0000-000000000000")
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
                "params":{
                    "name":"ctx.lsp_execute_command",
                    "arguments":{"path":"src/lib.rs","command":"ctx.test.fixAll","arguments":[]}
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
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("\"ok\": true"));
            got_call = true;
            break;
        }
    }
    assert!(got_call, "did not receive tools/call response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_lsp_semantic_tokens_full_calls_daemon_http() {
    let app = Router::new()
        .route(
            "/api/lsp/semantic_tokens/full",
            post(|| async { Json(json!({"resultId":"1","data":[0,0,5,0,0]})) }),
        )
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
        .env("CTX_SESSION_ID", "00000000-0000-0000-0000-000000000000")
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
                "params":{
                    "name":"ctx.lsp_semantic_tokens_full",
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
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("\"resultId\": \"1\""));
            got_call = true;
            break;
        }
    }
    assert!(got_call, "did not receive tools/call response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_lsp_code_actions_by_diagnostic_plan_calls_daemon_http() {
    let app = Router::new()
        .route(
            "/api/lsp/code_actions/by_diagnostic/plan",
            post(|| async {
                Json(json!([
                    {"id":"00000000-0000-0000-0000-000000000001","title":"Fix","diff":"diff --git a/x b/x","remaining_hunks":1}
                ]))
            }),
        )
        .layer(ServiceBuilder::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
        .env("CTX_SESSION_ID", "00000000-0000-0000-0000-000000000000")
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
                "params":{
                    "name":"ctx.lsp_code_actions_by_diagnostic_plan",
                    "arguments":{"path":"src/lib.rs","diagnostic":{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"x"}}
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
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("\"diff\""));
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

    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", format!("http://{}", addr))
        .env("CTX_SESSION_ID", "00000000-0000-0000-0000-000000000000")
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
                    "name":"ctx.lsp_rename_plan",
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
                    "name":"ctx.apply_edit_plan",
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
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(3) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            assert!(text.contains("\"remaining_hunks\": 0"));
            got_apply = true;
            break;
        }
    }
    assert!(got_apply, "did not receive tools/call apply response");

    let body = rename_body_tx
        .lock()
        .await
        .clone()
        .expect("expected rename body");
    assert_eq!(
        body.get("session_id").and_then(|v| v.as_str()),
        Some("00000000-0000-0000-0000-000000000000"),
        "expected session_id from env forwarded"
    );

    let _ = child.kill().await;
}
