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

async fn setup_state() -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
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
    (data_dir, state, app)
}

async fn create_session(
    app: axum::Router,
    repo: &Path,
) -> (axum::Router, context_core::models::Session) {
    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": repo.to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    if status != StatusCode::OK {
        panic!(
            "create workspace failed: status={status} body={}",
            String::from_utf8_lossy(&body)
        );
    }
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
        .body(Body::from(
            json!({"provider_id":"fake","model_id":"fake"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: context_core::models::Session = serde_json::from_slice(&body).unwrap();
    (app, session)
}

#[tokio::test]
async fn buffer_open_update_and_conflict() {
    let (_data_dir, state, app) = setup_state().await;
    let repo = setup_git_repo().await;
    let (app, session) = create_session(app, repo.path()).await;

    // open buffer (session scoped)
    let req = Request::builder()
        .method("POST")
        .uri("/api/buffers/open")
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
    let opened: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let buffer_id = opened
        .get("buffer_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // update buffer (autosave)
    let new_text = "pub fn ok() {}\npub fn added() {}\n";
    let req = Request::builder()
        .method("POST")
        .uri("/api/buffers/update")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "buffer_id": buffer_id,
                "version": 2,
                "text": new_text
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // simulate external change on disk, then expect conflict
    let wt = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .unwrap();
    let wt_root = std::path::PathBuf::from(wt.root_path);
    std::fs::write(wt_root.join("src/lib.rs"), "external\n").unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/api/buffers/update")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "buffer_id": opened.get("buffer_id").unwrap(),
                "version": 3,
                "text": "local\n"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // close buffer
    let req = Request::builder()
        .method("POST")
        .uri("/api/buffers/close")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "buffer_id": opened.get("buffer_id").unwrap(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
