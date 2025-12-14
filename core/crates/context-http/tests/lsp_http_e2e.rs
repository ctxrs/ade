use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::json;
use tokio::process::Command;
use tower::ServiceExt;

use context_http::api;
use context_http::daemon::AppState;
use context_lsp::LspManagerConfig;
use context_store::Store;

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
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn ok() {}\n").unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "ws"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

#[tokio::test]
async fn lsp_diagnostics_endpoint_returns_diagnostics() {
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let lsp_server = env!("CARGO_BIN_EXE_context-http-lsp-test-server").to_string();
    let state = Arc::new(AppState::new_with_lsp_config_and_flags(
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

    let repo = setup_git_repo().await;

    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: context_core::models::Workspace = serde_json::from_slice(&body).unwrap();

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
    let task: context_core::models::Task = serde_json::from_slice(&body).unwrap();

    // list tracks
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/tasks/{}/tracks", task.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let tracks: Vec<context_core::models::Track> = serde_json::from_slice(&body).unwrap();
    assert_eq!(tracks.len(), 1);
    let track = &tracks[0];

    // create session
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tracks/{}/sessions", track.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"provider_id":"fake","model_id":"fake"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: context_core::models::Session = serde_json::from_slice(&body).unwrap();

    let wt = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .unwrap();
    let wt_root = std::path::PathBuf::from(wt.root_path);
    tokio::fs::create_dir_all(wt_root.join("src")).await.unwrap();
    tokio::fs::write(wt_root.join("src/lib.rs"), "pub fn ok() {}\n")
        .await
        .unwrap();

    // call LSP diagnostics (session-scoped)
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/diagnostics")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let diags: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(diags.len(), 1);
    assert!(
        diags[0]
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("Intentional diagnostic"),
        "unexpected diagnostics payload: {}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test]
async fn lsp_status_endpoint_returns_expected_shape() {
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let lsp_server = env!("CARGO_BIN_EXE_context-http-lsp-test-server").to_string();
    let state = Arc::new(AppState::new_with_lsp_config_and_flags(
        data_dir.path().to_path_buf(),
        store.clone(),
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
        LspManagerConfig {
            enabled: true,
            rust_command: lsp_server.clone(),
            rust_args: vec![],
            ts_command: lsp_server.clone(),
            ts_args: vec![],
            py_command: lsp_server.clone(),
            py_args: vec![],
            go_command: lsp_server.clone(),
            go_args: vec![],
            diagnostics_wait: Duration::from_secs(2),
            ..Default::default()
        },
        true,
    ));
    let app = api::router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/lsp/status")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v.get("enabled").and_then(|x| x.as_bool()), Some(true));
    assert_eq!(
        v.get("edit_plans_enabled").and_then(|x| x.as_bool()),
        Some(true)
    );
    let servers = v.get("servers").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    assert_eq!(servers.len(), 4);
    for s in servers {
        assert!(s.get("language").and_then(|x| x.as_str()).unwrap_or("").len() > 0);
        assert!(s.get("command").and_then(|x| x.as_str()).unwrap_or("").len() > 0);
        assert!(s.get("found").and_then(|x| x.as_bool()).is_some());
    }
}

#[tokio::test]
async fn lsp_semantic_endpoints_return_payloads() {
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let lsp_server = env!("CARGO_BIN_EXE_context-http-lsp-test-server").to_string();
    let state = Arc::new(AppState::new_with_lsp_config_and_flags(
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

    let repo = setup_git_repo().await;

    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: context_core::models::Workspace = serde_json::from_slice(&body).unwrap();

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
    let task: context_core::models::Task = serde_json::from_slice(&body).unwrap();

    // list tracks
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/tasks/{}/tracks", task.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let tracks: Vec<context_core::models::Track> = serde_json::from_slice(&body).unwrap();
    assert_eq!(tracks.len(), 1);
    let track = &tracks[0];

    // create session
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tracks/{}/sessions", track.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"provider_id":"fake","model_id":"fake"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: context_core::models::Session = serde_json::from_slice(&body).unwrap();

    let wt = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .unwrap();
    let wt_root = std::path::PathBuf::from(wt.root_path);
    tokio::fs::create_dir_all(wt_root.join("src")).await.unwrap();
    tokio::fs::write(wt_root.join("src/lib.rs"), "pub fn ok() {}\n")
        .await
        .unwrap();

    let pos_req = |path: &str| {
        json!({
            "session_id": session.id.0.to_string(),
            "path": path,
            "line": 0,
            "character": 0
        })
    };

    for endpoint in [
        "/api/lsp/hover",
        "/api/lsp/signature_help",
        "/api/lsp/completion",
        "/api/lsp/type_definition",
        "/api/lsp/implementation",
    ] {
        let req = Request::builder()
            .method("POST")
            .uri(endpoint)
            .header("content-type", "application/json")
            .body(Body::from(pos_req("src/lib.rs").to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "endpoint {endpoint} failed");
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v != serde_json::Value::Null, "endpoint {endpoint} returned null");
    }
}

#[tokio::test]
async fn lsp_text_only_agent_endpoints_return_payloads() {
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let lsp_server = env!("CARGO_BIN_EXE_context-http-lsp-test-server").to_string();
    let state = Arc::new(AppState::new_with_lsp_config_and_flags(
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
            execute_commands_enabled: true,
            execute_command_allowlist: vec!["context.test.fixAll".to_string()],
            ..Default::default()
        },
        false,
    ));
    let app = api::router(state.clone());

    let repo = setup_git_repo().await;

    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: context_core::models::Workspace = serde_json::from_slice(&body).unwrap();

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
    let task: context_core::models::Task = serde_json::from_slice(&body).unwrap();

    // list tracks
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/tasks/{}/tracks", task.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let tracks: Vec<context_core::models::Track> = serde_json::from_slice(&body).unwrap();
    assert_eq!(tracks.len(), 1);
    let track = &tracks[0];

    // create session
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tracks/{}/sessions", track.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"provider_id":"fake","model_id":"fake"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: context_core::models::Session = serde_json::from_slice(&body).unwrap();

    let wt = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .unwrap();
    let wt_root = std::path::PathBuf::from(wt.root_path);
    tokio::fs::create_dir_all(wt_root.join("src")).await.unwrap();
    tokio::fs::write(wt_root.join("src/lib.rs"), "pub fn ok() {}\n")
        .await
        .unwrap();

    let pos_req = |path: &str| {
        json!({
            "session_id": session.id.0.to_string(),
            "path": path,
            "line": 0,
            "character": 0
        })
    };

    // Get a completion item and resolve it.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/completion")
        .header("content-type", "application/json")
        .body(Body::from(pos_req("src/lib.rs").to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let completion: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let item = completion
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/completion/resolve")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "item": item
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Inlay hints.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/inlay_hints")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "start_line": 0,
                "start_character": 0,
                "end_line": 0,
                "end_character": 1
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Document highlight.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/document_highlight")
        .header("content-type", "application/json")
        .body(Body::from(pos_req("src/lib.rs").to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Selection ranges.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/selection_ranges")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "positions": [{ "line": 0, "character": 0 }]
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Call hierarchy prepare + incoming/outgoing.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/call_hierarchy/prepare")
        .header("content-type", "application/json")
        .body(Body::from(pos_req("src/lib.rs").to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let item = items
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    for endpoint in ["/api/lsp/call_hierarchy/incoming", "/api/lsp/call_hierarchy/outgoing"] {
        let req = Request::builder()
            .method("POST")
            .uri(endpoint)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "session_id": session.id.0.to_string(),
                    "path": "src/lib.rs",
                    "item": item.clone()
                })
                .to_string(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "endpoint {endpoint} failed");
    }

    // Code lens + resolve.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/code_lens")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let lenses: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let lens = lenses
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/code_lens/resolve")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "item": lens
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // prepareRename.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/prepare_rename")
        .header("content-type", "application/json")
        .body(Body::from(pos_req("src/lib.rs").to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // document links + resolve.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/document_links")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let links: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let link = links
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/document_links/resolve")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "item": link
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // semantic tokens (agent-only consumers for now).
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/semantic_tokens/full")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // type hierarchy prepare + supertypes/subtypes.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/type_hierarchy/prepare")
        .header("content-type", "application/json")
        .body(Body::from(pos_req("src/lib.rs").to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let item = items
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    for endpoint in ["/api/lsp/type_hierarchy/supertypes", "/api/lsp/type_hierarchy/subtypes"] {
        let req = Request::builder()
            .method("POST")
            .uri(endpoint)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "session_id": session.id.0.to_string(),
                    "path": "src/lib.rs",
                    "item": item.clone()
                })
                .to_string(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "endpoint {endpoint} failed");
    }

    // executeCommand (captures applyEdit).
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/execute_command")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "command": "context.test.fixAll",
                "arguments": []
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v.get("workspace_edit").is_some(),
        "expected workspace_edit field"
    );
}
