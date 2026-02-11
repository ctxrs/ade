mod common;

use std::path::Path;

use axum::http::{Method, StatusCode};
use ctx_core::models::Worktree;
use serde_json::Value;

#[tokio::test]
async fn session_diff_endpoints_return_no_repo_unavailable() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state);

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "diff").await;
    let session = common::create_session(&app, task.id.0, "fake", "fake-model").await;
    let session_id = session.id;

    let (worktree_status, worktree): (StatusCode, Worktree) = common::json_request(
        &app,
        Method::GET,
        format!("/api/worktrees/{}", session.worktree_id.0),
        None,
    )
    .await;
    assert_eq!(worktree_status, StatusCode::OK);
    let worktree_root = Path::new(&worktree.root_path);
    let git_path = worktree_root.join(".git");
    let metadata = tokio::fs::metadata(&git_path)
        .await
        .expect("worktree .git should exist");
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&git_path).await.unwrap();
    } else {
        tokio::fs::remove_file(&git_path).await.unwrap();
    }

    let (diff_status, diff): (StatusCode, Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/sessions/{}/diff", session_id.0),
        None,
    )
    .await;
    assert_eq!(diff_status, StatusCode::OK);
    assert_eq!(diff.get("available").and_then(Value::as_bool), Some(false));
    assert_eq!(
        diff.get("unavailable_reason").and_then(Value::as_str),
        Some("no_repo")
    );
    assert_eq!(diff.get("diff").and_then(Value::as_str), Some(""));

    let (summary_status, summary): (StatusCode, Value) = common::json_request(
        &app,
        Method::GET,
        format!("/api/sessions/{}/diff/summary", session_id.0),
        None,
    )
    .await;
    assert_eq!(summary_status, StatusCode::OK);
    assert_eq!(
        summary.get("available").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        summary.get("unavailable_reason").and_then(Value::as_str),
        Some("no_repo")
    );
    assert_eq!(summary.get("file_count").and_then(Value::as_i64), Some(0));
    assert_eq!(
        summary.get("line_additions").and_then(Value::as_i64),
        Some(0)
    );
    assert_eq!(
        summary.get("line_deletions").and_then(Value::as_i64),
        Some(0)
    );
}
