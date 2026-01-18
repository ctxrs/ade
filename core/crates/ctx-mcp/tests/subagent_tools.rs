use std::time::Duration;

use axum::{routing::get, routing::post, Json, Router};
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
    let invocation_id = "subagent-1";

    let app = Router::new()
        .route(
            &format!("/api/mcp/sessions/{}/agent_init", parent_id),
            post(move |Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["agents"][0]["prompt"], "check foo");
                assert_eq!(body["response_mode"], "await");
                assert_eq!(body["tool_call_id"], "tool-1");
                Json(json!({
                    "invocation_id": invocation_id,
                    "status": "completed",
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
        .route(
            &format!("/api/sessions/{}/subagent_invocations", parent_id),
            get(move || async move {
                Json(json!([
                    {
                        "id": invocation_id,
                        "tool_call_id": "tool-1",
                        "parent_session_id": parent_id,
                        "requested_count": 1,
                        "status": "completed",
                        "created_at": "2024-01-01T00:00:00Z",
                        "updated_at": "2024-01-01T00:00:00Z",
                        "children": [{
                            "invocation_id": invocation_id,
                            "child_session_id": child_id,
                            "run_id": "11111111-1111-1111-1111-111111111111",
                            "position": 0,
                            "status": "completed",
                            "prompt_length": 3,
                            "created_at": "2024-01-01T00:00:00Z",
                            "updated_at": "2024-01-01T00:00:00Z"
                        }]
                    }
                ]))
            }),
        )
        .route(
            &format!("/api/subagent_invocations/{}", invocation_id),
            get(move || async move {
                Json(json!({
                    "id": invocation_id,
                    "tool_call_id": "tool-1",
                    "parent_session_id": parent_id,
                    "requested_count": 1,
                    "status": "completed",
                    "created_at": "2024-01-01T00:00:00Z",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "children": [{
                        "invocation_id": invocation_id,
                        "child_session_id": child_id,
                        "run_id": "11111111-1111-1111-1111-111111111111",
                        "position": 0,
                        "status": "completed",
                        "prompt_length": 3,
                        "created_at": "2024-01-01T00:00:00Z",
                        "updated_at": "2024-01-01T00:00:00Z"
                    }]
                }))
            }),
        )
        .route(
            &format!("/api/mcp/sessions/{}/subagent_wait", parent_id),
            post(move |Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["invocation_id"], invocation_id);
                Json(json!({
                    "invocation_id": invocation_id,
                    "status": "completed",
                    "results": [{
                        "session_id": child_id,
                        "label": "Audit FooAPI",
                        "provider_id": "codex",
                        "model_id": "gpt-5.2",
                        "status": "completed",
                        "content": "final"
                    }]
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
                    "response_mode":"await",
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
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let mut subagent_group_id: Option<String> = None;
    let mut subagent_id: Option<String> = None;
    let init_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < init_deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(3) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            let payload: serde_json::Value = serde_json::from_str(text).unwrap();
            subagent_group_id = payload
                .get("subagent_group_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            subagent_id = payload
                .get("results")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("subagent_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            assert!(text.contains("found 3 usages"));
            break;
        }
    }

    let subagent_group_id = subagent_group_id.expect("missing subagent_group_id");
    let subagent_id = subagent_id.expect("missing subagent_id");

    for msg in [
        json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params":{
                "name":"ctx.agent_reply",
                "arguments":{
                    "subagent_id": subagent_id,
                    "prompt":"summarize output"
                }
            }
        }),
        json!({
            "jsonrpc":"2.0",
            "id":5,
            "method":"tools/call",
            "params":{
                "name":"ctx.subagent_invocations_list",
                "arguments":{}
            }
        }),
        json!({
            "jsonrpc":"2.0",
            "id":6,
            "method":"tools/call",
            "params":{
                "name":"ctx.subagent_invocation_get",
                "arguments":{
                    "subagent_group_id": subagent_group_id
                }
            }
        }),
        json!({
            "jsonrpc":"2.0",
            "id":7,
            "method":"tools/call",
            "params":{
                "name":"ctx.subagent_wait",
                "arguments":{
                    "subagent_group_id": subagent_group_id
                }
            }
        }),
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let mut got_reply = false;
    let mut got_list = false;
    let mut got_get = false;
    let mut got_wait = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        match v.get("id").and_then(|id| id.as_i64()) {
            Some(4) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains("summary"));
                got_reply = true;
            }
            Some(5) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains(&subagent_group_id));
                got_list = true;
            }
            Some(6) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains("completed"));
                got_get = true;
            }
            Some(7) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains("final"));
                got_wait = true;
            }
            _ => {}
        }
        if got_reply && got_list && got_get && got_wait {
            break;
        }
    }

    assert!(got_reply, "did not receive agent_reply response");
    assert!(
        got_list,
        "did not receive subagent_invocations_list response"
    );
    assert!(got_get, "did not receive subagent_invocation_get response");
    assert!(got_wait, "did not receive subagent_wait response");
    let _ = child.kill().await;
}
