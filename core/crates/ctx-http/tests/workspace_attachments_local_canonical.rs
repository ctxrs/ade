mod common;

use axum::http::{Method, StatusCode};
use ctx_core::models::{WorkspaceAttachment, WorkspaceAttachmentKind};
use serde_json::json;

#[tokio::test]
async fn workspace_attachments_are_db_canonical_and_ignore_repo_file() {
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let cfg_path = repo.path().join(".ctx").join("attachments.toml");
    tokio::fs::create_dir_all(cfg_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&cfg_path, "not-valid-toml = [")
        .await
        .unwrap();
    let before = tokio::fs::read_to_string(&cfg_path).await.unwrap();

    let data_root = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_root.path()).await;
    let state = common::build_state(
        data_root.path(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state);

    let workspace = common::create_workspace(&app, repo.path(), "ws").await;

    let (create_status, created): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        Some(json!({
            "kind": "reference_repo",
            "name": "ref-fixture",
            "source": repo.path().to_string_lossy().to_string()
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK);
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].kind, WorkspaceAttachmentKind::ReferenceRepo);
    assert_eq!(created[0].name, "ref-fixture");

    let after_create = tokio::fs::read_to_string(&cfg_path).await.unwrap();
    assert_eq!(after_create, before);

    let (sync_status, synced): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments/sync", workspace.id.0),
        Some(json!({ "refresh": true })),
    )
    .await;
    assert_eq!(sync_status, StatusCode::OK);
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].name, "ref-fixture");

    let after_sync = tokio::fs::read_to_string(&cfg_path).await.unwrap();
    assert_eq!(after_sync, before);

    let (list_status, listed): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::GET,
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        None,
    )
    .await;
    assert_eq!(list_status, StatusCode::OK);
    assert_eq!(listed.len(), 1);

    let (delete_status, remaining): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::DELETE,
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        Some(json!({
            "kind": "reference_repo",
            "name": "ref-fixture"
        })),
    )
    .await;
    assert_eq!(delete_status, StatusCode::OK);
    assert!(remaining.is_empty());

    let after_delete = tokio::fs::read_to_string(&cfg_path).await.unwrap();
    assert_eq!(after_delete, before);
}
