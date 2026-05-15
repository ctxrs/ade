use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use ctx_core::ids::WorktreeId;
use ctx_daemon::test_support::TestDaemon;
use tower::ServiceExt;
use uuid::Uuid;

fn test_router(daemon: &TestDaemon) -> axum::Router {
    crate::api::router(crate::api::RouteHandles::from_daemon_handle(
        daemon.handle(),
    ))
}

#[tokio::test]
async fn get_worktree_returns_live_root_for_bound_sandbox_worktree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_root = temp.path().join("repo");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let daemon = TestDaemon::new_for_test(
        temp.path().to_path_buf(),
        "http://127.0.0.1:4310".to_string(),
    )
    .await
    .expect("create daemon");
    let host_root = temp.path().join("managed-worktree");
    std::fs::create_dir_all(&host_root).expect("create managed worktree");
    let worktree = daemon
        .seed_sandbox_bound_worktree_for_test("ws", &workspace_root, &host_root, "/ctx/ws")
        .await
        .expect("seed sandbox-bound worktree");

    let app = test_router(&daemon);
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/worktrees/{}", worktree.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let response: Worktree = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        response.root_path,
        format!("/ctx/ws/worktrees/{}", worktree.id.0)
    );
    assert_eq!(response.id, worktree.id);
    assert_eq!(response.workspace_id, worktree.workspace_id);
    daemon.request_shutdown();
}

#[tokio::test]
async fn missing_worktree_routes_return_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon = TestDaemon::new_for_test(
        temp.path().to_path_buf(),
        "http://127.0.0.1:4310".to_string(),
    )
    .await
    .expect("create daemon");
    let missing_worktree_id = WorktreeId(Uuid::new_v4()).0.to_string();
    let app = test_router(&daemon);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/worktrees/{missing_worktree_id}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/worktrees/{missing_worktree_id}/bootstrap/logs"
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    daemon.request_shutdown();
}
