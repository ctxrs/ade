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

