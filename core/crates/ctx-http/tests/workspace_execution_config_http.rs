mod common;

use axum::http::{Method, StatusCode};
use serde_json::Value;

#[tokio::test]
async fn workspace_registration_rejects_invalid_root_path() {
    let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
    let app = fixture.router();

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        "/api/workspaces",
        Some(serde_json::json!({"root_path": " ", "name": "ws"})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:#?}");
    assert!(
        body.get("error")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("root_path is required")),
        "invalid workspace root should surface registration guidance: {body:#?}"
    );
}

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

#[tokio::test]
async fn workspace_execution_config_preserves_invalid_id_error_contract() {
    let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
    let app = fixture.router();

    let (get_status, get_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::GET,
        "/api/workspaces/not-a-workspace/execution_config",
        None,
    )
    .await;
    assert_eq!(get_status, StatusCode::BAD_REQUEST);
    assert_eq!(
        get_body.get("error").and_then(Value::as_str),
        Some("invalid workspace id")
    );

    let (post_status, post_body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        "/api/workspaces/not-a-workspace/execution_config",
        Some(serde_json::json!({
            "environment": "host"
        })),
    )
    .await;
    assert_eq!(post_status, StatusCode::BAD_REQUEST);
    assert_eq!(
        post_body.get("error").and_then(Value::as_str),
        Some("invalid workspace id")
    );
}

#[tokio::test]
async fn workspace_execution_config_rejects_invalid_request_values() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
    let app = fixture.router();
    let workspace = common::create_workspace(&app, repo.path(), "ws").await;

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/execution_config", workspace.id.0),
        Some(serde_json::json!({
            "environment": "container",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:#?}");
    assert!(
        body.get("error")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("invalid environment")),
        "invalid environment should be rejected: {body:#?}"
    );

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/execution_config", workspace.id.0),
        Some(serde_json::json!({
            "environment": "host",
            "network_mode": "public",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:#?}");
    assert!(
        body.get("error")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("invalid network_mode")),
        "invalid network mode should be rejected: {body:#?}"
    );
}
