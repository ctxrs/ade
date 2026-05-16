mod common;

use axum::http::{Method, StatusCode};
use serde_json::Value;

#[tokio::test]
async fn workspace_execution_config_fails_closed_on_invalid_runtime_settings() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
    let daemon = &fixture.daemon;
    let app = fixture.router();
    let workspace = common::create_workspace(&app, repo.path(), "ws").await;

    daemon
        .seed_invalid_workspace_runtime_settings_document_for_test(
            workspace.id,
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
