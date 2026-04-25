use super::*;

#[tokio::test]
async fn mcp_subagent_tools_call_daemon_http() {
    let parent_id = "00000000-0000-0000-0000-000000000001";

    let app = Router::new()
        .route(
            &format!("/api/mcp/sessions/{parent_id}/subagent_init"),
            post(move |Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["agents"][0]["prompt"], "check foo");
                assert_eq!(body["worktree"], "inherit");
                assert_eq!(body["tool_call_id"], "tool-1");
                Json(json!({
                    "status": "running",
                    "results": [{
                        "label": "Audit FooAPI",
                        "status": "running"
                    }]
                }))
            }),
        )
        .route(
            &format!("/api/mcp/sessions/{parent_id}/subagent_reply"),
            post(move |Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["label"], "Audit FooAPI");
                assert_eq!(body["prompt"], "summarize output");
                Json(json!({
                    "label": "Audit FooAPI",
                    "status": "running"
                }))
            }),
        )
        .route(
            &format!("/api/mcp/sessions/{parent_id}/subagent_list"),
            get(move || async move {
                Json(json!([
                    {
                        "label": "Audit FooAPI",
                        "status": "active"
                    }
                ]))
            }),
        )
        .route(
            &format!("/api/mcp/sessions/{parent_id}/subagent_wait"),
            post(move |Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["label"], "Audit FooAPI");
                Json(json!({
                    "status": "completed",
                    "results": [{
                        "label": "Audit FooAPI",
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
        .env("CTX_DAEMON_URL", format!("http://{addr}"))
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
                "name":"ctx.subagent_init",
                "_meta":{"toolCallId":"tool-1"},
                "arguments":{
                    "worktree":"inherit",
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

    let init_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < init_deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(3) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            let payload: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(payload["results"][0]["label"], "Audit FooAPI");
            break;
        }
    }

    for msg in [
        json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params":{
                "name":"ctx.subagent_reply",
                "arguments":{
                    "label": "Audit FooAPI",
                    "prompt":"summarize output"
                }
            }
        }),
        json!({
            "jsonrpc":"2.0",
            "id":5,
            "method":"tools/call",
            "params":{
                "name":"ctx.subagent_list",
                "arguments":{}
            }
        }),
        json!({
            "jsonrpc":"2.0",
            "id":7,
            "method":"tools/call",
            "params":{
                "name":"ctx.subagent_wait",
                "arguments":{
                    "label": "Audit FooAPI"
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
                assert!(text.contains("\"status\": \"running\""));
                got_reply = true;
            }
            Some(5) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains("Audit FooAPI"));
                got_list = true;
            }
            Some(7) => {
                let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
                assert!(text.contains("final"));
                got_wait = true;
            }
            _ => {}
        }
        if got_reply && got_list && got_wait {
            break;
        }
    }

    assert!(got_reply, "did not receive subagent_reply response");
    assert!(got_list, "did not receive subagent_list response");
    assert!(got_wait, "did not receive subagent_wait response");
    let _ = child.kill().await;
}

