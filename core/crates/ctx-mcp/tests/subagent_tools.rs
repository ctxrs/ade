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
async fn mcp_subagent_tools_call_daemon_http() {
    let parent_id = "00000000-0000-0000-0000-000000000001";
    let child_id = "00000000-0000-0000-0000-000000000002";

    let app = Router::new()
        .route(
            &format!("/api/mcp/sessions/{}/agent_init", parent_id),
            post(move |Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["agents"][0]["prompt"], "check foo");
                assert_eq!(body["tool_call_id"], "tool-1");
                Json(json!({
                    "results": [{
                        "session_id": child_id,
                        "label": "Audit FooAPI",
                        "provider_id": "codex",
                        "model_id": "gpt-5.2",
                        "status": "completed",
                        "content": "found 3 usages"
                    }]
                }))
            }),
        )
        .route(
            &format!("/api/mcp/sessions/{}/agent_reply", parent_id),
            post(move |Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["session_id"], child_id);
                assert_eq!(body["prompt"], "summarize output");
                Json(json!({
                    "session_id": child_id,
                    "status": "completed",
                    "content": "summary"
                }))
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
        .env("CTX_SESSION_ID", parent_id)
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
                "name":"ctx.agent_init",
                "_meta":{"toolCallId":"tool-1"},
                "arguments":{
                    "agents":[
                        {
                            "prompt":"check foo",
                            "label":"Audit FooAPI",
                            "harness":"codex",
                            "model":"gpt-5.2"
                        }
                    ]
                }
            }
        }),
        json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params":{
                "name":"ctx.agent_reply",
                "arguments":{
                    "session_id": child_id,
                    "prompt":"summarize output"
                }
            }
        }),
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let mut got_init = false;
    let mut got_reply = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        match v.get("id").and_then(|id| id.as_i64()) {
            Some(3) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains("found 3 usages"));
                got_init = true;
            }
            Some(4) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains("summary"));
                got_reply = true;
            }
            _ => {}
        }
        if got_init && got_reply {
            break;
        }
    }

    assert!(got_init, "did not receive agent_init response");
    assert!(got_reply, "did not receive agent_reply response");
    let _ = child.kill().await;
}
