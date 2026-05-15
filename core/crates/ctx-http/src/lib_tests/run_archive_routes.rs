use super::*;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{RunArchiveIngestBatch, RunArchiveIngestCursor, RunArchiveIngestScope};

#[tokio::test]
async fn run_archive_routes_build_and_acknowledge_org_visible_batch() {
    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let state = test_daemon(data_dir.path(), stores, None);
    let app = test_router(&state);

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let fixture = state
        .seed_org_visible_run_archive_fixture_for_test(workspace.id, git_repo.path())
        .await
        .unwrap();
    let run_id = fixture.run_id;

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_batch?max_items=25",
            workspace.id.0, run_id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let batch: Option<RunArchiveIngestBatch> = serde_json::from_slice(&body).unwrap();
    let batch = batch.expect("org-visible run should produce archive batch");
    assert_eq!(batch.run.id, run_id);
    assert_eq!(batch.run.workspace_id, workspace.id);
    assert_eq!(batch.scope, RunArchiveIngestScope::Evidence);
    assert_eq!(batch.messages.len(), 1);
    let serialized = serde_json::to_string(&batch).unwrap();
    assert!(!serialized.contains("/Users/example-user"));
    assert!(!serialized.contains("sk-test-placeholder-1234567890"));

    let mut tampered_batch = batch.clone();
    tampered_batch.to.session_event_seq += 100;
    tampered_batch.to.audit_event_seq += 100;
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_ack",
            workspace.id.0, run_id.0
        ))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&tampered_batch).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    let missing_workspace_id = WorkspaceId::new();
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_batch?max_items=25",
            missing_workspace_id.0, run_id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let mut missing_workspace_batch = batch.clone();
    missing_workspace_batch.run.workspace_id = missing_workspace_id;
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_ack",
            missing_workspace_id.0, run_id.0
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&missing_workspace_batch).unwrap(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_ack",
            workspace.id.0, run_id.0
        ))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&batch).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let cursor: RunArchiveIngestCursor = serde_json::from_slice(&body).unwrap();
    assert_eq!(cursor.run_id, run_id);
    assert_eq!(cursor.watermark, batch.to);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_batch",
            workspace.id.0, run_id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let batch: Option<RunArchiveIngestBatch> = serde_json::from_slice(&body).unwrap();
    assert!(batch.is_none());
}
