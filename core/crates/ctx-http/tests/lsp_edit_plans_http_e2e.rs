use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::json;
use tokio::process::Command;
use tower::ServiceExt;

use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_lsp::LspManagerConfig;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

fn fake_providers() -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    providers
}

fn file_uri(path: &Path) -> String {
    url::Url::from_file_path(path).unwrap().to_string()
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
) -> (ctx_core::models::Session, std::path::PathBuf) {
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
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    // create task (auto worktree)
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

    // create session
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"fake","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: ctx_core::models::Session = serde_json::from_slice(&body).unwrap();

    let store = state.store_for_session(session.id).await.unwrap();
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .unwrap();
    let wt_root = std::path::PathBuf::from(wt.root_path);
    tokio::fs::create_dir_all(wt_root.join("src"))
        .await
        .unwrap();
    tokio::fs::write(wt_root.join("src/lib.rs"), "pub fn ok() {}\n")
        .await
        .unwrap();

    (session, wt_root)
}

async fn setup_state_and_app(
    lsp_edit_plans_enabled: bool,
) -> (tempfile::TempDir, Arc<AppState>, axum::Router) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let lsp_server = env!("CARGO_BIN_EXE_ctx-http-lsp-test-server").to_string();
    let state = Arc::new(AppState::new_with_lsp_config_and_flags(
        data_dir.path().to_path_buf(),
        stores,
        fake_providers(),
        "http://127.0.0.1:4399".to_string(),
        None,
        LspManagerConfig {
            enabled: true,
            rust_command: lsp_server,
            rust_args: vec![],
            diagnostics_wait: Duration::from_secs(2),
            execute_commands_enabled: true,
            execute_command_allowlist: vec!["ctx.test.fixAll".to_string()],
            ..Default::default()
        },
        lsp_edit_plans_enabled,
    ));
    let app = api::router(state.clone());
    (data_dir, state, app)
}

#[tokio::test]
async fn edit_plan_persists_across_restart_and_discards() {
    let (data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;

    let (session, _wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

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
        .unwrap()
        .to_string();

    let plan_path = data_dir
        .path()
        .join("edit_plans")
        .join(format!("{plan_id}.json"));
    assert!(
        plan_path.exists(),
        "expected plan persisted at {}, but it does not exist",
        plan_path.to_string_lossy()
    );

    // "Restart" daemon by creating a new AppState against the same data_root.
    drop(app);
    drop(state);

    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let lsp_server = env!("CARGO_BIN_EXE_ctx-http-lsp-test-server").to_string();
    let state2 = Arc::new(AppState::new_with_lsp_config_and_flags(
        data_dir.path().to_path_buf(),
        stores,
        fake_providers(),
        "http://127.0.0.1:4399".to_string(),
        None,
        LspManagerConfig {
            enabled: true,
            rust_command: lsp_server,
            rust_args: vec![],
            diagnostics_wait: Duration::from_secs(2),
            execute_commands_enabled: true,
            execute_command_allowlist: vec!["ctx.test.fixAll".to_string()],
            ..Default::default()
        },
        true,
    ));
    let app2 = api::router(state2.clone());

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/worktrees/{}/edit_plans",
            session.worktree_id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app2.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let plans: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0]
            .get("id")
            .and_then(|v| v.as_str().or_else(|| v.get("0").and_then(|x| x.as_str()))),
        Some(plan_id.as_str())
    );

    // Discard.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/discard", plan_id))
        .body(Body::from("{}"))
        .unwrap();
    let res = app2.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(!plan_path.exists(), "expected plan file deleted");
}

#[tokio::test]
async fn stale_plan_is_rejected_on_apply() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;

    let (session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

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

    // Mutate the file out-of-band to make the plan stale.
    tokio::fs::write(wt_root.join("src/lib.rs"), "pub fn changed() {}\n")
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"action":"accept","patch": diff}).to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn lsp_rename_plan_create_and_apply() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;

    let (session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

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
        .body(Body::from(
            json!({"action":"accept","patch": diff}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    if res.status() != StatusCode::OK {
        let status = res.status();
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let patch_snippet = &diff[..std::cmp::min(diff.len(), 2000)];
        panic!(
            "apply failed with {}: {}\npatch:\n{}",
            status,
            String::from_utf8_lossy(&body),
            patch_snippet
        );
    }

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs"))
        .await
        .unwrap();
    assert!(
        updated.contains("rename: better"),
        "file not updated:\n{updated}"
    );

    // Plan removed after apply.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/worktrees/{}/edit_plans",
            session.worktree_id.0
        ))
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
    let (session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

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
    let action = actions
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .expect("expected one action");

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
        .body(Body::from(
            json!({"action":"accept","patch": diff}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    if res.status() != StatusCode::OK {
        let status = res.status();
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let patch_snippet = &diff[..std::cmp::min(diff.len(), 2000)];
        panic!(
            "apply failed with {}: {}\npatch:\n{}",
            status,
            String::from_utf8_lossy(&body),
            patch_snippet
        );
    }

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs"))
        .await
        .unwrap();
    assert!(updated.contains("// TODO"), "file not updated:\n{updated}");
}

#[tokio::test]
async fn lsp_code_action_plan_supports_command_only_embedded_edit() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;
    let (session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

    let file = wt_root.join("src/lib.rs");
    let uri = file_uri(&file);
    let action = json!({
        "title": "Insert CMD",
        "kind": "quickfix",
        "command": {
            "title": "Insert CMD",
            "command": "ctx.test.insertCmd",
            "arguments": [{
                "edit": {
                    "changes": {
                        uri: [{
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 0}
                            },
                            "newText": "// CMD\n"
                        }]
                    }
                }
            }]
        }
    });

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
    assert!(diff.contains("// CMD"), "unexpected diff:\n{diff}");

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"action":"accept","patch": diff}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    if res.status() != StatusCode::OK {
        let status = res.status();
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let patch_snippet = &diff[..std::cmp::min(diff.len(), 2000)];
        panic!(
            "apply failed with {}: {}\npatch:\n{}",
            status,
            String::from_utf8_lossy(&body),
            patch_snippet
        );
    }

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs"))
        .await
        .unwrap();
    assert!(updated.contains("// CMD"), "file not updated:\n{updated}");
}

#[tokio::test]
async fn lsp_code_action_plan_supports_workspace_edit_file_ops() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;
    let (session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

    let create_path = wt_root.join("src/created.rs");
    let rename_from = wt_root.join("src/rename_from.rs");
    let rename_to = wt_root.join("src/rename_to.rs");
    let delete_path = wt_root.join("src/delete_me.rs");
    tokio::fs::write(&rename_from, "fn from() {}\n")
        .await
        .unwrap();
    tokio::fs::write(&delete_path, "fn delete_me() {}\n")
        .await
        .unwrap();

    let create_uri = file_uri(&create_path);
    let rename_from_uri = file_uri(&rename_from);
    let rename_to_uri = file_uri(&rename_to);
    let delete_uri = file_uri(&delete_path);

    let action = json!({
        "title": "File ops",
        "kind": "quickfix",
        "edit": {
            "documentChanges": [
                { "kind": "create", "uri": create_uri },
                {
                    "textDocument": { "uri": create_uri, "version": null },
                    "edits": [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0}
                        },
                        "newText": "fn created() {}\n"
                    }]
                },
                { "kind": "rename", "oldUri": rename_from_uri, "newUri": rename_to_uri },
                {
                    "textDocument": { "uri": rename_to_uri, "version": null },
                    "edits": [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0}
                        },
                        "newText": "// renamed\n"
                    }]
                },
                { "kind": "delete", "uri": delete_uri }
            ]
        }
    });

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
    assert!(diff.contains("created.rs"), "unexpected diff:\n{diff}");
    assert!(diff.contains("rename_to.rs"), "unexpected diff:\n{diff}");
    assert!(diff.contains("delete_me.rs"), "unexpected diff:\n{diff}");

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"action":"accept","patch": diff}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    if res.status() != StatusCode::OK {
        let status = res.status();
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let patch_snippet = &diff[..std::cmp::min(diff.len(), 2000)];
        panic!(
            "apply failed with {}: {}\npatch:\n{}",
            status,
            String::from_utf8_lossy(&body),
            patch_snippet
        );
    }

    assert!(
        !rename_from.exists(),
        "expected {} removed",
        rename_from.display()
    );
    assert!(
        !delete_path.exists(),
        "expected {} removed",
        delete_path.display()
    );
    assert!(
        create_path.exists(),
        "expected {} created",
        create_path.display()
    );
    assert!(
        rename_to.exists(),
        "expected {} created",
        rename_to.display()
    );

    let created = tokio::fs::read_to_string(&create_path).await.unwrap();
    assert!(
        created.contains("fn created()"),
        "unexpected create contents:\n{created}"
    );
    let renamed = tokio::fs::read_to_string(&rename_to).await.unwrap();
    assert!(
        renamed.contains("// renamed"),
        "unexpected rename contents:\n{renamed}"
    );
    assert!(
        renamed.contains("fn from"),
        "unexpected rename contents:\n{renamed}"
    );
}

#[tokio::test]
async fn lsp_organize_imports_plan_create_and_apply() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;
    let (session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

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
    assert!(
        diff.contains("organize imports"),
        "unexpected diff:\n{diff}"
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"action":"accept","patch": diff}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs"))
        .await
        .unwrap();
    assert!(
        updated.contains("organize imports"),
        "file not updated:\n{updated}"
    );
}

#[tokio::test]
async fn lsp_execute_command_plan_create_and_apply() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;
    let (session, wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/execute_command/plan")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "command": "ctx.test.fixAll",
                "arguments": []
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
    assert!(diff.contains("execCommand"), "unexpected diff:\n{diff}");

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/apply", plan_id))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"action":"accept","patch": diff}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let updated = tokio::fs::read_to_string(wt_root.join("src/lib.rs"))
        .await
        .unwrap();
    assert!(
        updated.contains("execCommand"),
        "file not updated:\n{updated}"
    );
}

#[tokio::test]
async fn lsp_code_actions_by_diagnostic_plan_creates_plans() {
    let (_data_dir, state, app) = setup_state_and_app(true).await;
    let repo = setup_git_repo().await;

    let (session, _wt_root) = create_workspace_task_session(&app, &state, repo.path()).await;

    // Fetch diagnostics, pick the first one.
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
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let diags: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let diag = diags.first().cloned().expect("expected a diagnostic");

    // Request ranked quick-fix plans for that diagnostic.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/code_actions/by_diagnostic/plan")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session.id.0.to_string(),
                "path": "src/lib.rs",
                "diagnostic": diag
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let plans: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(!plans.is_empty(), "expected at least one plan");
    assert!(
        plans[0]
            .get("diff")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("diff --git"),
        "expected plan diff"
    );
}
