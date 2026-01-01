use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::json;
use tokio::process::Command;
use tower::ServiceExt;

use ctx_core::models::SessionEventType;
use ctx_providers::tier1::Tier1AcpAdapter;
use ctx_store::Store;

use ctx_http::api;
use ctx_http::daemon::AppState;

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

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    std::fs::write(root.join("note.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn build_state_with_real_providers(data_root: PathBuf, store: Store) -> Arc<AppState> {
    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("codex".into(), Arc::new(Tier1AcpAdapter::codex()));
    providers.insert("claude".into(), Arc::new(Tier1AcpAdapter::claude()));
    providers.insert("gemini".into(), Arc::new(Tier1AcpAdapter::gemini()));

    let state = Arc::new(AppState::new(
        data_root,
        store,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    state.start_workspace_catchup_listener();
    state
}

async fn setup_state_with_real_providers() -> (tempfile::TempDir, Store, Arc<AppState>) {
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let state = build_state_with_real_providers(data_dir.path().to_path_buf(), store.clone()).await;
    (data_dir, store, state)
}

async fn create_session_with_provider(
    app: &mut axum::Router,
    git_repo_root: &Path,
    provider_id: &str,
) -> ctx_core::models::Session {
    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo_root.to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    // create task (auto track + worktree)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    // fetch workspace catchup to locate the default track
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/workspaces/{}/catchup", ws.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let snapshot: ctx_core::models::WorkspaceCatchupSnapshot =
        serde_json::from_slice(&body).unwrap();
    let track = snapshot
        .active
        .tasks
        .iter()
        .find(|summary| summary.task.id == task.id)
        .and_then(|summary| summary.tracks.first())
        .expect("default track missing");

    // create session
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tracks/{}/sessions", track.track.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":provider_id,"model_id":"default"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: ctx_core::models::Session = serde_json::from_slice(&body).unwrap();

    session
}

async fn post_message(app: &mut axum::Router, session_id: &str, content: &str) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{session_id}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(json!({ "content": content }).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

async fn wait_for_tool_events(store: &Store, session_id: ctx_core::ids::SessionId) {
    let mut attempts = 0;
    loop {
        let events = store.list_session_events(session_id).await.unwrap();
        if events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::ToolCall))
            && events
                .iter()
                .any(|e| matches!(e.event_type, SessionEventType::ToolResult))
        {
            if events
                .iter()
                .any(|e| matches!(e.event_type, SessionEventType::Error))
            {
                panic!("saw Error event(s): {events:#?}");
            }
            break;
        }
        attempts += 1;
        if attempts > 600 {
            panic!("timed out waiting for tool events");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_for_new_tool_events(
    store: &Store,
    session_id: context_core::ids::SessionId,
    prev_len: usize,
) -> Vec<context_core::models::SessionEvent> {
    let mut attempts = 0;
    loop {
        let events = store.list_session_events(session_id).await.unwrap();
        if events.len() > prev_len {
            let new_events = &events[prev_len..];
            if new_events
                .iter()
                .any(|e| matches!(e.event_type, SessionEventType::ToolCall))
                && new_events
                    .iter()
                    .any(|e| matches!(e.event_type, SessionEventType::ToolResult))
            {
                if new_events
                    .iter()
                    .any(|e| matches!(e.event_type, SessionEventType::Error))
                {
                    panic!("saw Error event(s): {new_events:#?}");
                }
                return new_events.to_vec();
            }
        }
        attempts += 1;
        if attempts > 600 {
            panic!("timed out waiting for new tool events");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

const PROMPT: &str = r#"
Use your shell tool to run: ls -1
Do not guess the output; you MUST run the command.
Reply with just: done
"#;

const RESUME_PROMPT: &str = r#"
Use your shell tool to run: pwd
Do not guess the output; you MUST run the command.
Reply with just: done
"#;

#[tokio::test]
#[ignore]
async fn runner_codex_real_acp_produces_tool_events() {
    which::which("codex-acp").expect("codex-acp binary not found on PATH");
    let git_repo = setup_git_repo().await;
    let (_data_dir, store, state) = setup_state_with_real_providers().await;
    let mut app = api::router(state);

    let session = create_session_with_provider(&mut app, git_repo.path(), "codex").await;
    post_message(&mut app, &session.id.0.to_string(), PROMPT).await;
    wait_for_tool_events(&store, session.id).await;
}

#[tokio::test]
#[ignore]
async fn runner_claude_real_acp_produces_tool_events() {
    which::which("claude-code-acp").expect("claude-code-acp binary not found on PATH");
    let git_repo = setup_git_repo().await;
    let (_data_dir, store, state) = setup_state_with_real_providers().await;
    let mut app = api::router(state);

    let session = create_session_with_provider(&mut app, git_repo.path(), "claude").await;
    post_message(&mut app, &session.id.0.to_string(), PROMPT).await;
    wait_for_tool_events(&store, session.id).await;
}

#[tokio::test]
#[ignore]
async fn runner_gemini_real_acp_produces_tool_events() {
    which::which("gemini").expect("gemini binary not found on PATH");
    let git_repo = setup_git_repo().await;
    let (_data_dir, store, state) = setup_state_with_real_providers().await;
    let mut app = api::router(state);

    let session = create_session_with_provider(&mut app, git_repo.path(), "gemini").await;
    post_message(&mut app, &session.id.0.to_string(), PROMPT).await;
    wait_for_tool_events(&store, session.id).await;
}

#[tokio::test]
#[ignore]
async fn runner_codex_real_acp_resumes_without_rehydrate() {
    which::which("codex-acp").expect("codex-acp binary not found on PATH");
    let git_repo = setup_git_repo().await;
    let (data_dir, store, state) = setup_state_with_real_providers().await;
    let mut app = api::router(state);

    let session = create_session_with_provider(&mut app, git_repo.path(), "codex").await;
    post_message(&mut app, &session.id.0.to_string(), PROMPT).await;
    wait_for_tool_events(&store, session.id).await;

    let session_row = store
        .get_session(session.id)
        .await
        .unwrap()
        .expect("missing session");
    let provider_ref = session_row
        .provider_session_ref
        .clone()
        .expect("provider_session_ref missing after first run");

    let events_before = store.list_session_events(session.id).await.unwrap();
    let prev_len = events_before.len();

    drop(app);

    let state = build_state_with_real_providers(data_dir.path().to_path_buf(), store.clone()).await;
    let mut app = api::router(state);

    post_message(&mut app, &session.id.0.to_string(), RESUME_PROMPT).await;
    let new_events = wait_for_new_tool_events(&store, session.id, prev_len).await;

    if new_events.iter().any(|ev| {
        matches!(ev.event_type, SessionEventType::Notice)
            && ev.payload_json.get("kind").and_then(|v| v.as_str()) == Some("rehydrate")
    }) {
        panic!("unexpected rehydrate notice on resume: {new_events:#?}");
    }

    let init_event = new_events
        .iter()
        .rev()
        .find(|ev| matches!(ev.event_type, SessionEventType::Init))
        .expect("missing Init event after resume");
    assert_eq!(
        init_event
            .payload_json
            .get("acp_session_id")
            .and_then(|v| v.as_str()),
        Some(provider_ref.as_str()),
        "resume should keep the same provider session id"
    );
    assert_eq!(
        init_event
            .payload_json
            .get("resumed")
            .and_then(|v| v.as_bool()),
        Some(true),
        "resume should be reported as resumed"
    );
}
