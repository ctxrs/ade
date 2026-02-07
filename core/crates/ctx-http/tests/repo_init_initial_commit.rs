mod common;

use axum::body::Body;
use axum::http::Request;
use serde_json::json;

#[tokio::test]
async fn repo_init_creates_initial_commit() {
    let data_root = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_root.path()).await;
    let state = common::build_state(
        data_root.path(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state);

    let dir = tempfile::tempdir().unwrap();
    let repo_path = dir.path().join("repo");

    let req = Request::builder()
        .method("POST")
        .uri("/api/repo/init")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "path": repo_path.to_string_lossy().to_string(),
                "allow_existing": false,
            })
            .to_string(),
        ))
        .unwrap();

    let (status, _resp): (axum::http::StatusCode, serde_json::Value) =
        common::oneshot_json(&app, req).await;
    assert_eq!(status, axum::http::StatusCode::OK);

    // Ensure the repo has a HEAD commit, which is required for worktree-based sessions.
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&repo_path)
        .arg("rev-parse")
        .arg("--verify")
        .arg("HEAD")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "expected HEAD commit, got stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
