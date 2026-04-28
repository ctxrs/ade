mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, WorkspaceAttachment, WorkspaceAttachmentKind,
    WorkspaceAttachmentStatus,
};
use ctx_http::attachments;
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

#[tokio::test]
async fn workspace_attachments_sync_heals_stale_pending_when_materialized_exists() {
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;

    let data_root = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_root.path()).await;
    let state = common::build_state(
        data_root.path(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let workspace = common::create_workspace(&app, repo.path(), "ws").await;
    let attachment = attachments::upsert_workspace_attachment(
        state.as_ref(),
        workspace.id,
        attachments::AttachmentConfig {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "ref-fixture".to_string(),
            source: repo.path().to_string_lossy().to_string(),
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: Some(AttachmentMode::Ro),
            update_policy: Some(AttachmentUpdatePolicy::Manual),
        },
    )
    .await
    .unwrap();

    let materialized = data_root
        .path()
        .join("attachments")
        .join("reference-repos")
        .join("checkouts")
        .join(attachment.id.0.to_string())
        .join("default");
    tokio::fs::create_dir_all(&materialized).await.unwrap();
    tokio::fs::write(materialized.join("README.md"), "cached\n")
        .await
        .unwrap();

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    store
        .update_workspace_attachment_status(
            attachment.id,
            WorkspaceAttachmentStatus::Pending,
            None,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let (sync_status, synced): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments/sync", workspace.id.0),
        Some(json!({ "refresh": false })),
    )
    .await;
    assert_eq!(sync_status, StatusCode::OK);
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].status, WorkspaceAttachmentStatus::Ready);
    assert!(synced[0].last_sync_at.is_some());
}

#[tokio::test]
async fn deleting_workspace_attachment_cancels_inflight_materialization() {
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let script_path = repo
        .path()
        .join(".ctx")
        .join("scripts")
        .join("slow-docs.py");
    tokio::fs::create_dir_all(script_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(
        &script_path,
        r#"
import pathlib
import sys
import time

dest = pathlib.Path(sys.argv[1])
dest.mkdir(parents=True, exist_ok=True)
marker = pathlib.Path(".ctx/doc-mirror-started")
marker.parent.mkdir(parents=True, exist_ok=True)
marker.write_text("started\n", encoding="utf-8")
time.sleep(2.0)
dest.mkdir(parents=True, exist_ok=True)
(dest / "index.md").write_text("done\n", encoding="utf-8")
"#,
    )
    .await
    .unwrap();

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
    let started_marker = repo.path().join(".ctx").join("doc-mirror-started");
    let mount_path = repo
        .path()
        .join(".ctx")
        .join("attachments")
        .join("docs")
        .join("slow-docs");

    let (create_status, created): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        Some(json!({
            "kind": "doc_mirror",
            "name": "slow-docs",
            "source": ".ctx/scripts/slow-docs.py"
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK);
    let attachment = created
        .into_iter()
        .find(|entry| entry.name == "slow-docs")
        .unwrap();
    let materialized_root = data_root
        .path()
        .join("attachments")
        .join("doc-mirrors")
        .join(attachment.id.0.to_string());

    let (sync_status, synced): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments/sync", workspace.id.0),
        Some(json!({ "refresh": true })),
    )
    .await;
    assert_eq!(sync_status, StatusCode::OK);
    assert_eq!(synced.len(), 1);

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::fs::metadata(&started_marker).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("doc mirror script never started");

    let (delete_status, remaining): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::DELETE,
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        Some(json!({
            "kind": "doc_mirror",
            "name": "slow-docs"
        })),
    )
    .await;
    assert_eq!(delete_status, StatusCode::OK);
    assert!(remaining.is_empty());

    tokio::time::sleep(Duration::from_secs(3)).await;

    assert!(!materialized_root.exists());
    assert!(!mount_path.exists());
}

#[tokio::test]
async fn workspace_attachments_reject_doc_mirror_scripts_outside_workspace_root() {
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let outside = tempfile::tempdir().unwrap();
    let marker = outside.path().join("ran.txt");
    let script_path = outside.path().join("outside-docs.py");
    tokio::fs::write(
        &script_path,
        format!(
            "from pathlib import Path\nPath({:?}).write_text('ran', encoding='utf-8')\n",
            marker.to_string_lossy()
        ),
    )
    .await
    .unwrap();

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
            "kind": "doc_mirror",
            "name": "outside-docs",
            "source": script_path.to_string_lossy().to_string()
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK);
    assert_eq!(created.len(), 1);

    let (sync_status, synced): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments/sync", workspace.id.0),
        Some(json!({ "refresh": true })),
    )
    .await;
    assert_eq!(sync_status, StatusCode::OK);
    assert_eq!(synced.len(), 1);

    let attachment = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_list_status, listed): (StatusCode, Vec<WorkspaceAttachment>) = common::json_request(
                &app,
                Method::GET,
                format!("/api/workspaces/{}/attachments", workspace.id.0),
                None,
            )
            .await;
            let current = listed
                .into_iter()
                .find(|entry| entry.name == "outside-docs")
                .expect("attachment should still exist");
            if current.status == WorkspaceAttachmentStatus::Error {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("doc mirror attachment never reached error state");

    assert_eq!(attachment.status, WorkspaceAttachmentStatus::Error);
    assert!(
        attachment
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("must stay within workspace root"),
        "expected workspace-root error, got {:?}",
        attachment.error_message
    );
    assert!(!marker.exists(), "outside script should never execute");
}
