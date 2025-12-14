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

async fn create_workspace_task_session(
    app: &axum::Router,
    state: &Arc<AppState>,
    repo_root: &Path,
) -> (context_core::models::Track, context_core::models::Session, std::path::PathBuf) {
    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": repo_root.to_string_lossy(),
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
    let track = tracks[0].clone();

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

    (track, session, wt_root)
}

async fn setup_state_and_app(
    lsp_edit_plans_enabled: bool,
) -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    let lsp_server = env!("CARGO_BIN_EXE_context-http-lsp-test-server").to_string();
    let state = Arc::new(AppState::new_with_lsp_config_and_flags(
        data_dir.path().to_path_buf(),
        store,
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
        lsp_edit_plans_enabled,
    ));
    let app = api::router(state.clone());
    (data_dir, state, app)
}

#[tokio::test]
async fn lsp_rename_plan_create_and_apply() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;

    let (track, session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/rename/plan")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "line": 0,
                "character": 0,
                "new_name": "better"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let summary: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let plan_id = summary
        .get("id")
        .and_then(|v| v.as_str().or_else(|| v.get("0").and_then(|x| x.as_str())))
        .unwrap();
    let diff = summary.get("diff").and_then(|v| v.as_str()).unwrap();
    assert!(diff.contains("rename: better"), "unexpected diff:\n{diff}");

    // Apply all.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(json!({"action":"accept","patch": diff}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs")).await.unwrap();
    assert!(updated.contains("rename: better"), "file not updated:\n{updated}");

    // Plan removed after apply.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/tracks/{}/edit_plans", track.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let plans: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(plans.is_empty(), "expected edit plans empty, got {plans:?}");
}

#[tokio::test]
async fn lsp_code_action_plan_create_and_apply() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;
    let (_track, session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

    // Get code actions from the LSP endpoint so the JSON matches lsp-types expectations.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/code_actions")
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
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let actions: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let action = actions.as_array().and_then(|a| a.first()).cloned().expect("expected one action");

    // Create plan.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/code_actions/plan")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "action": action
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let summary: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let plan_id = summary
        .get("id")
        .and_then(|v| v.as_str().or_else(|| v.get("0").and_then(|x| x.as_str())))
        .unwrap();
    let diff = summary.get("diff").and_then(|v| v.as_str()).unwrap();
    assert!(diff.contains("// TODO"), "unexpected diff:\n{diff}");

    // Apply.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(json!({"action":"accept","patch": diff}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs")).await.unwrap();
    assert!(updated.contains("// TODO"), "file not updated:\n{updated}");
}

#[tokio::test]
async fn lsp_organize_imports_plan_create_and_apply() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;
    let (_track, session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/organize_imports/plan")
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
    let summary: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let plan_id = summary
        .get("id")
        .and_then(|v| v.as_str().or_else(|| v.get("0").and_then(|x| x.as_str())))
        .unwrap();
    let diff = summary.get("diff").and_then(|v| v.as_str()).unwrap();
    assert!(diff.contains("organize imports"), "unexpected diff:\n{diff}");

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(json!({"action":"accept","patch": diff}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs")).await.unwrap();
    assert!(
        updated.contains("organize imports"),
        "file not updated:\n{updated}"
    );
}
