mod common;

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use ctx_core::models::Session;
use ctx_http::daemon::AppState;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

#[tokio::test]
async fn create_session_accepts_legacy_codex_alias() {
    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("codex-crp".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state);

    let workspace = common::create_workspace(&app, repo.path(), "ws").await;
    let (task_status, task): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(serde_json::json!({
            "title": "codex-alias",
            "create_default_session": false
        })),
    )
    .await;
    assert_eq!(task_status, StatusCode::OK);
    let task_id = task
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("task id")
        .to_string();

    let (session_status, session): (StatusCode, Session) = common::json_request(
        &app,
        Method::POST,
        format!("/api/tasks/{task_id}/sessions"),
        Some(serde_json::json!({
            "provider_id": "codex",
            "model_id": "gpt-5.5/medium",
            "execution_environment": "host"
        })),
    )
    .await;
    assert_eq!(session_status, StatusCode::OK);
    assert_eq!(session.provider_id, "codex-crp");
}
