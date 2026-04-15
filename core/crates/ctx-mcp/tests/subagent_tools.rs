use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::get, routing::post, Json, Router};
use ctx_core::models::{ExecutionEnvironment, Session, Task, VcsKind, Workspace, Worktree};
use ctx_http::daemon::AppState;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tower::ServiceBuilder;
use uuid::Uuid;

fn mcp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ctx-mcp")
}

async fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn run_git_output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

async fn init_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    tokio::fs::write(root.join("README.md"), "ok\n")
        .await
        .unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn setup_daemon_backed_parent_session() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
    String,
    String,
) {
    let repo = init_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        base_url.clone(),
        None,
    ));
    state.providers.statuses.lock().await.insert(
        "fake".into(),
        FakeProviderAdapter::new().inspect().await.unwrap(),
    );

    let app = ctx_http::api::router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let workspace: Workspace = stores
        .global()
        .create_workspace(
            "test".into(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = stores.workspace(workspace.id).await.unwrap();
    let base_commit = run_git_output(repo.path(), &["rev-parse", "HEAD"]).await;
    let worktree: Worktree = store
        .create_worktree(
            workspace.id,
            repo.path().to_string_lossy().to_string(),
            base_commit,
            None,
        )
        .await
        .unwrap();
    let task: Task = store
        .create_task(workspace.id, "task".into(), None)
        .await
        .unwrap();
    let session: Session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".into(),
            "fake-model".into(),
            "assistant".into(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();

    (repo, data_dir, state, base_url, session.id.0.to_string())
}

async fn wait_for_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    response_id: i64,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let Some(line) = reader.next_line().await.unwrap() else {
            break;
        };
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        if value.get("id").and_then(|id| id.as_i64()) == Some(response_id) {
            return value;
        }
    }
    panic!("timed out waiting for response id {response_id}");
}

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

#[tokio::test]
async fn mcp_subagent_tools_work_end_to_end_against_real_daemon_router() {
    let (_repo, _data_dir, _state, base_url, parent_id) =
        setup_daemon_backed_parent_session().await;

    let mut child = Command::new(mcp_bin())
        .arg("--stdio")
        .env("CTX_DAEMON_URL", &base_url)
        .env("CTX_SESSION_ID", &parent_id)
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
                "arguments":{
                    "worktree":"inherit",
                    "agents":[
                        {
                            "prompt":"reply with exactly OK",
                            "label":"Audit FooAPI",
                            "harness":"fake",
                            "model":"fake-model"
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

    let init_response = wait_for_response(&mut reader, 3).await;
    let init_text = init_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let init_payload: serde_json::Value = serde_json::from_str(init_text).unwrap();
    assert_eq!(init_payload["status"], "running");
    assert_eq!(init_payload["results"][0]["label"], "Audit FooAPI");
    assert_eq!(init_payload["results"][0]["status"], "running");

    stdin
        .write_all(
            json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"ctx.subagent_wait",
                    "arguments":{
                        "label":"Audit FooAPI"
                    }
                }
            })
            .to_string()
            .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let wait_response = wait_for_response(&mut reader, 4).await;
    let wait_text = wait_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let wait_payload: serde_json::Value = serde_json::from_str(wait_text).unwrap();
    assert_eq!(wait_payload["status"], "completed");
    assert_eq!(wait_payload["results"][0]["label"], "Audit FooAPI");
    assert_eq!(wait_payload["results"][0]["status"], "completed");
    assert!(wait_payload["results"][0]["content"]
        .as_str()
        .unwrap_or("")
        .contains("done: reply with exactly OK"));

    let _ = child.kill().await;
}

#[tokio::test]
#[ignore = "Requires live provider credentials and a built ctx-mcp binary on the product path."]
async fn live_provider_parent_can_invoke_real_subagent_via_ctx_mcp() {
    let provider_id = std::env::var("CTX_LIVE_PROVIDER_ID").ok();
    let model_id = std::env::var("CTX_LIVE_MODEL_ID").ok();
    if provider_id.is_none() || model_id.is_none() {
        eprintln!(
            "skipping: set CTX_LIVE_PROVIDER_ID and CTX_LIVE_MODEL_ID to run live subagent canary"
        );
        return;
    }
    let provider_id = provider_id.unwrap();
    let model_id = model_id.unwrap();
    if !matches!(
        provider_id.as_str(),
        "codex" | "codex-crp" | "claude" | "claude-crp"
    ) {
        eprintln!("skipping: live subagent canary only supports codex/claude providers");
        return;
    }

    std::env::set_var("CTX_MCP_COMMAND", mcp_bin());

    let repo = init_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let adapter: Arc<dyn ProviderAdapter> = match provider_id.as_str() {
        "codex" | "codex-crp" => Arc::new(ctx_providers::crp::Tier1CrpAdapter::codex()),
        "claude" | "claude-crp" => Arc::new(ctx_providers::crp::Tier1CrpAdapter::claude()),
        _ => unreachable!(),
    };
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(provider_id.clone(), adapter);

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        base_url.clone(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let workspace: Workspace = client
        .post(format!("{base_url}/api/workspaces"))
        .json(&json!({
            "root_path": repo.path().to_string_lossy(),
            "name": "ws"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let task: Task = client
        .post(format!(
            "{base_url}/api/workspaces/{}/tasks",
            workspace.id.0
        ))
        .json(&json!({ "title": "t1" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session: Session = client
        .post(format!("{base_url}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({ "provider_id": provider_id, "model_id": model_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let token = format!("CTX_SUBAGENT_LIVE_OK_{}", Uuid::new_v4());
    client
        .post(format!("{base_url}/api/sessions/{}/messages", session.id.0))
        .json(&json!({
            "content": format!(
                "Use ctx.subagent_init to launch exactly one subagent labeled ping. Ask it to reply with exactly {token}. Then use ctx.subagent_wait for label ping. After the subagent completes, reply with exactly {token} and nothing else."
            )
        }))
        .send()
        .await
        .unwrap();

    let store = state.store_for_session(session.id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        if events
            .iter()
            .any(|event| matches!(event.event_type, ctx_core::models::SessionEventType::Done))
        {
            break;
        }
        if events.iter().any(|event| {
            matches!(
                event.event_type,
                ctx_core::models::SessionEventType::Error
                    | ctx_core::models::SessionEventType::AuthRequired
            )
        }) {
            panic!("live subagent canary saw terminal error/auth-required events: {events:#?}");
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for live subagent canary completion");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let subagents = store.list_subagent_sessions(session.id).await.unwrap();
    assert!(
        !subagents.is_empty(),
        "expected live provider to create at least one subagent session"
    );

    let events = store.list_session_events(session.id).await.unwrap();
    let assistant_messages = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type,
                ctx_core::models::SessionEventType::AssistantMessageInserted
            )
        })
        .filter_map(|event| {
            event
                .payload_json
                .get("content")
                .and_then(|value| value.as_str())
        })
        .collect::<Vec<_>>();
    assert!(
        assistant_messages
            .iter()
            .any(|message| message.contains(&token)),
        "expected final assistant message containing {token}; saw {assistant_messages:#?}"
    );
}
