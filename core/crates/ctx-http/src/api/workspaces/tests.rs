use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{VcsKind, Worktree};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn workspace_registry_and_harness_status_routes_return_seeded_workspace() {
    let fixture = crate::test_support::TestDaemonFixture::new("http://127.0.0.1:4310").await;
    let daemon = fixture.daemon();
    let workspace_root = fixture.data_root().join("repo");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let workspace = daemon
        .seed_workspace_for_test("workspace", &workspace_root, VcsKind::Git)
        .await
        .expect("seed workspace");

    let app = fixture.router();
    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let list: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let workspace_id = workspace.id.0.to_string();
    assert!(
        list.iter()
            .any(|item| item.get("id").and_then(serde_json::Value::as_str)
                == Some(workspace_id.as_str())),
        "workspace list did not include seeded workspace: {list:#?}"
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/workspaces/{}", workspace.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        fetched.get("root_path").and_then(serde_json::Value::as_str),
        Some(workspace.root_path.as_str())
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/harness_container",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, serde_json::Value::Null);

    daemon.request_shutdown();
}

#[tokio::test]
async fn invalid_worktree_routes_return_bad_request() {
    let fixture = crate::test_support::TestDaemonFixture::new("http://127.0.0.1:4310").await;
    let daemon = fixture.daemon();
    let app = fixture.router();

    let req = Request::builder()
        .method("GET")
        .uri("/api/worktrees/not-a-worktree")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let req = Request::builder()
        .method("GET")
        .uri("/api/worktrees/not-a-worktree/bootstrap/logs")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    daemon.request_shutdown();
}

#[tokio::test]
async fn get_worktree_returns_live_root_for_bound_sandbox_worktree() {
    let fixture = crate::test_support::TestDaemonFixture::new("http://127.0.0.1:4310").await;
    let daemon = fixture.daemon();
    let workspace_root = fixture.data_root().join("repo");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let host_root = fixture.data_root().join("managed-worktree");
    std::fs::create_dir_all(&host_root).expect("create managed worktree");
    let worktree = daemon
        .seed_sandbox_bound_worktree_for_test("ws", &workspace_root, &host_root, "/ctx/ws")
        .await
        .expect("seed sandbox-bound worktree");

    let app = fixture.router();
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
    let fixture = crate::test_support::TestDaemonFixture::new("http://127.0.0.1:4310").await;
    let daemon = fixture.daemon();
    let missing_worktree_id = WorktreeId(Uuid::new_v4()).0.to_string();
    let app = fixture.router();

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
