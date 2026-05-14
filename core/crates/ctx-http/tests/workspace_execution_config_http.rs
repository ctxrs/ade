mod common;

use axum::http::{Method, StatusCode};
use ctx_daemon::daemon::DaemonState;
use ctx_store::StoreManager;
use serde_json::Value;
use std::sync::Arc;

fn build_state(data_root: &std::path::Path, stores: StoreManager) -> Arc<DaemonState> {
    common::build_state(
        data_root,
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    )
}

#[tokio::test]
async fn workspace_execution_config_fails_closed_on_invalid_runtime_settings() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = build_state(data_dir.path(), stores);
    let app = common::router(state.clone());
    let workspace = common::create_workspace(&app, repo.path(), "ws").await;

    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("load workspace store");
    store
        .upsert_runtime_settings_document(
            1,
            r#"{
  "execution": {
    "environment": 7
  }
}"#,
        )
        .await
        .expect("write malformed runtime settings");

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/workspaces/{}/execution_config", workspace.id.0),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:#?}");
    assert!(
        body.get("error")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("workspace runtime settings")),
        "invalid workspace runtime settings should be surfaced instead of falling back: {body:#?}"
    );
}
