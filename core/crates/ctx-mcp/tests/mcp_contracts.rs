use std::time::Duration;

use axum::{extract::Path, routing::get, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tower::ServiceBuilder;

fn mcp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ctx-mcp")
}

#[tokio::test]
async fn mcp_tools_list_hides_lsp_by_default() {
    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", "http://127.0.0.1:9")
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
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_list = false;
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let tools = v["result"]["tools"].as_array().expect("tools array");
            let names: Vec<String> = tools
                .iter()
                .filter_map(|t| {
                    t.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            assert!(
                !names.iter().any(|n| n.starts_with("lsp_")),
                "expected lsp_* tools to be hidden by default"
            );
            assert!(
                !names.iter().any(|n| matches!(
                    n.as_str(),
                    "list_edit_plans" | "get_edit_plan" | "apply_edit_plan" | "discard_edit_plan"
                )),
                "expected edit plan tools to be hidden by default"
            );
            got_list = true;
            break;
        }
    }

    assert!(got_list, "did not receive tools/list response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_subagent_tool_schemas_avoid_top_level_combinators() {
    let bin = mcp_bin();
    let mut child = Command::new(bin)
        .arg("--stdio")
        .env("CTX_DAEMON_URL", "http://127.0.0.1:9")
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
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_list = false;
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let tools = v["result"]["tools"].as_array().expect("tools array");
            for tool_name in ["subagent_wait", "subagent_interrupt"] {
                let tool = tools
                    .iter()
                    .find(|tool| tool.get("name").and_then(|name| name.as_str()) == Some(tool_name))
                    .unwrap_or_else(|| panic!("missing tool {tool_name}"));
                let schema = tool["inputSchema"].as_object().expect("inputSchema object");
                assert_eq!(
                    schema.get("type").and_then(|value| value.as_str()),
                    Some("object"),
                    "expected {tool_name} schema type=object"
                );
                for key in ["anyOf", "allOf", "oneOf", "not", "enum"] {
                    assert!(
                        !schema.contains_key(key),
                        "expected {tool_name} schema to avoid top-level {key}"
                    );
                }
            }
            got_list = true;
            break;
        }
    }

    assert!(got_list, "did not receive tools/list response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_list_workspaces_scrubs_internal_ids() {
    let app = Router::new()
        .route(
            "/api/workspaces",
            get(|| async {
                Json(json!([
                    {
                        "id": "ws-1",
                        "host_id": "host-1",
                        "name": "Alpha",
                        "root_path": "/tmp/alpha",
                        "created_at": "2026-01-01T00:00:00Z"
                    }
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
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    for msg in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{
                "name":"ctx.list_workspaces",
                "arguments":{}
            }
        }),
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_call = false;
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            let payload: Value = serde_json::from_str(text).unwrap();
            let items = payload.as_array().expect("expected workspace list");
            let item = items.first().expect("expected workspace entry");
            let obj = item.as_object().expect("workspace entry must be object");
            assert_eq!(obj.get("name").and_then(|v| v.as_str()), Some("Alpha"));
            assert_eq!(
                obj.get("root_path").and_then(|v| v.as_str()),
                Some("/tmp/alpha")
            );
            assert_eq!(obj.len(), 2, "expected only name + root_path");
            got_call = true;
            break;
        }
    }

    assert!(got_call, "did not receive list_workspaces response");
    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_merge_queue_submit_scrubs_internal_ids() {
    let body_tx = std::sync::Arc::new(tokio::sync::Mutex::new(None::<Value>));
    let body_tx2 = body_tx.clone();

    let app = Router::new()
        .route(
            "/api/merge-queue/entries",
            post(move |Json(body): Json<Value>| {
                let body_tx2 = body_tx2.clone();
                async move {
                    *body_tx2.lock().await = Some(body);
                    Json(json!({
                        "id":"entry-1",
                        "session_id":"sess-internal",
                        "workspace_id":"ws-1",
                        "worktree_id":"wt-1",
                        "task_id":"task-1",
                        "status":"queued",
                        "target_branch":"main",
                        "message":"merge it",
                        "meta": {
                            "session_id": "sess-internal",
                            "note": "keep"
                        }
                    }))
                }
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

    for msg in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{
                "name":"ctx.merge_queue_submit",
                "arguments":{
                    "target_branch":"main",
                    "message":"merge it"
                }
            }
        }),
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_call = false;
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            let payload: Value = serde_json::from_str(text).unwrap();
            let obj = payload.as_object().expect("response must be object");
            assert_eq!(obj.get("status").and_then(|v| v.as_str()), Some("queued"));
            assert_eq!(
                obj.get("target_branch").and_then(|v| v.as_str()),
                Some("main")
            );
            assert_eq!(
                obj.get("message").and_then(|v| v.as_str()),
                Some("merge it")
            );
            assert!(obj.get("id").is_none(), "internal id should be scrubbed");
            assert!(
                obj.get("session_id").is_none(),
                "internal session_id should be scrubbed"
            );
            assert!(
                obj.get("workspace_id").is_none(),
                "internal workspace_id should be scrubbed"
            );
            assert!(
                obj.get("worktree_id").is_none(),
                "internal worktree_id should be scrubbed"
            );
            assert!(
                obj.get("task_id").is_none(),
                "internal task_id should be scrubbed"
            );
            let meta = obj
                .get("meta")
                .and_then(|v| v.as_object())
                .expect("missing meta");
            assert!(
                meta.get("session_id").is_none(),
                "nested session_id should be scrubbed"
            );
            assert_eq!(meta.get("note").and_then(|v| v.as_str()), Some("keep"));
            got_call = true;
            break;
        }
    }

    assert!(got_call, "did not receive merge_queue_submit response");

    let body = body_tx.lock().await.clone().expect("missing request body");
    assert_eq!(
        body.get("session_id").and_then(|v| v.as_str()),
        Some("00000000-0000-0000-0000-000000000000"),
        "expected session context to be passed to daemon"
    );

    let _ = child.kill().await;
}

#[tokio::test]
async fn mcp_oracle_forwards_prompt_and_overrides() {
    let body_tx = std::sync::Arc::new(tokio::sync::Mutex::new(None::<Value>));
    let body_tx2 = body_tx.clone();
    let temp_dir = std::env::temp_dir().join(format!("ctx-mcp-oracle-{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let prompt_path = temp_dir.join("prompt.txt");
    let response_path = temp_dir.join("response.txt");
    tokio::fs::write(&prompt_path, "hello").await.unwrap();
    let prompt_path_str = prompt_path.to_string_lossy().to_string();
    let response_path_str = response_path.to_string_lossy().to_string();

    let app = Router::new()
        .route(
            "/api/mcp/sessions/:id/oracle",
            post(move |Path(_id): Path<String>, Json(body): Json<Value>| {
                let body_tx2 = body_tx2.clone();
                async move {
                    *body_tx2.lock().await = Some(body);
                    Json(json!({
                        "model":"gpt-5.2-pro",
                        "reasoning_effort":"high",
                        "text":"ok"
                    }))
                }
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

    for msg in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{
                "name":"ctx.oracle",
                "arguments":{
                    "prompt_path": prompt_path_str.clone(),
                    "response_path": response_path_str.clone(),
                    "model":"gpt-5.2-pro",
                    "reasoning_effort":"high",
                    "max_output_tokens":123,
                    "timeout_ms":4567
                }
            }
        }),
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_call = false;
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let v: Value = serde_json::from_str(&line).unwrap();
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            let payload: Value = serde_json::from_str(text).unwrap();
            let obj = payload.as_object().expect("response must be object");
            assert_eq!(
                obj.get("prompt_path").and_then(|v| v.as_str()),
                Some(prompt_path_str.as_str())
            );
            assert_eq!(
                obj.get("response_path").and_then(|v| v.as_str()),
                Some(response_path_str.as_str())
            );
            assert_eq!(obj.get("prompt_bytes").and_then(|v| v.as_u64()), Some(5));
            assert_eq!(obj.get("response_bytes").and_then(|v| v.as_u64()), Some(2));
            let input_copy_path = obj
                .get("input_copy_path")
                .and_then(|v| v.as_str())
                .expect("missing input_copy_path");
            let copied_prompt = tokio::fs::read_to_string(input_copy_path).await.unwrap();
            assert_eq!(copied_prompt, "hello");
            let response_text = tokio::fs::read_to_string(&response_path).await.unwrap();
            assert_eq!(response_text, "ok");
            got_call = true;
            break;
        }
    }

    assert!(got_call, "did not receive oracle response");

    let body = body_tx.lock().await.clone().expect("missing request body");
    let obj = body.as_object().expect("oracle request must be object");
    assert_eq!(obj.get("prompt").and_then(|v| v.as_str()), Some("hello"));
    assert_eq!(
        obj.get("model").and_then(|v| v.as_str()),
        Some("gpt-5.2-pro")
    );
    assert_eq!(
        obj.get("reasoning_effort").and_then(|v| v.as_str()),
        Some("high")
    );
    assert_eq!(
        obj.get("max_output_tokens").and_then(|v| v.as_i64()),
        Some(123)
    );
    assert_eq!(obj.get("timeout_ms").and_then(|v| v.as_i64()), Some(4567));

    let _ = child.kill().await;
}
