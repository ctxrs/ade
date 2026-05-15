use std::ffi::OsStr;

use axum::http::{Method, StatusCode};
use serde_json::json;

mod common;

struct EnvVarGuard {
    key: String,
    prev: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &str, value: impl AsRef<OsStr>) -> Self {
        let prev = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key: key.to_string(),
            prev,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => unsafe {
                std::env::set_var(&self.key, value);
            },
            None => unsafe {
                std::env::remove_var(&self.key);
            },
        }
    }
}

#[tokio::test]
async fn dev_seed_session_transcript_populates_prior_turns() {
    let _dev_mode = EnvVarGuard::set("CTX_DEV_MODE", "1");

    let repo = common::init_git_repo(&[("README.md", "fixture\n")]).await;
    let fixture = common::fake_daemon_fixture("http://127.0.0.1:0").await;
    let app = fixture.router();

    let workspace = common::create_workspace(&app, repo.path(), "demo").await;
    let (task, session) = common::create_task_with_session(
        &app,
        workspace.id.0,
        "Ping Pong Demo",
        "fake",
        "fake-model",
    )
    .await;

    let (seed_status, seed_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/dev/sessions/{}/seed_transcript", session.id.0),
        Some(json!({
            "session_title": "Ping Pong Build",
            "task_title": "Ping Pong Demo",
            "turns": [
                {
                    "user": "Can you build a tiny browser game?",
                    "assistant": "Yes. I can put it in a single HTML file."
                },
                {
                    "user": "Keep it easy to show in a demo.",
                    "assistant": "Understood. I will keep the diff compact and visible."
                }
            ]
        })),
    )
    .await;
    assert_eq!(seed_status, StatusCode::OK, "{seed_body:#?}");
    assert_eq!(seed_body["seeded_turns"], json!(2));
    assert_eq!(seed_body["seeded_messages"], json!(4));

    let (head_status, head_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/sessions/{}/head?include_events=true", session.id.0),
        None,
    )
    .await;
    assert_eq!(head_status, StatusCode::OK, "{head_body:#?}");
    assert_eq!(head_body["session"]["title"], json!("Ping Pong Build"));
    assert_eq!(
        head_body["turns"].as_array().map(|items| items.len()),
        Some(2)
    );
    assert_eq!(
        head_body["messages"].as_array().map(|items| items.len()),
        Some(4)
    );
    let events = head_body["events"]
        .as_array()
        .expect("session events array");
    assert!(events
        .iter()
        .any(|event| event["event_type"] == "assistant_message_inserted"));
    assert!(events.iter().any(|event| event["event_type"] == "done"));

    let (snapshot_status, snapshot_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/workspaces/{}/active_snapshot", workspace.id.0),
        None,
    )
    .await;
    assert_eq!(snapshot_status, StatusCode::OK, "{snapshot_body:#?}");
    let active_tasks = snapshot_body["active"]["tasks"]
        .as_array()
        .expect("active snapshot tasks array");
    let seeded_task = active_tasks
        .iter()
        .find(|item| item["task"]["id"] == json!(task.id.0.to_string()))
        .expect("seeded task present in active snapshot");
    assert_eq!(
        seeded_task["task"]["primary_session_id"],
        json!(session.id.0.to_string())
    );
    assert!(
        seeded_task["primary_session"]["session"]["id"] == json!(session.id.0.to_string()),
        "seeded task should expose the seeded session as its primary session"
    );

    let (heads_status, heads_body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/workspaces/{}/active_heads", workspace.id.0),
        None,
    )
    .await;
    assert_eq!(heads_status, StatusCode::OK, "{heads_body:#?}");
    let seeded_head = heads_body["heads"]
        .as_array()
        .expect("workspace active heads array")
        .iter()
        .find(|item| item["session"]["id"] == json!(session.id.0.to_string()))
        .expect("seeded session head present in workspace active heads");
    assert_eq!(
        seeded_head["messages"].as_array().map(|items| items.len()),
        Some(4)
    );
}
