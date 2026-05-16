mod common;

use axum::http::{Method, StatusCode};
use serde_json::Value;

#[tokio::test]
async fn subagent_system_prompt_endpoint_returns_default_append() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
    let app = fixture.router();
    let ws = common::create_workspace(&app, repo.path(), "ws").await;

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/workspaces/{}/subagent_system_prompt", ws.id.0),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("default_append").and_then(Value::as_str),
        Some(
            "You are a subagent. The user messaging you is the primary agent who will provide your instructions."
        )
    );
    assert_eq!(
        body.get("effective_append").and_then(Value::as_str),
        Some(
            "You are a subagent. The user messaging you is the primary agent who will provide your instructions."
        )
    );
    assert_eq!(body.get("configured_append").and_then(Value::as_str), None);
    assert_eq!(body.get("source").and_then(Value::as_str), Some("default"));
}
