mod common;

use axum::http::{Method, StatusCode};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, WorkspaceAttachment, WorkspaceAttachmentKind,
    WorkspaceAttachmentStatus,
};
use ctx_http::attachments;
use ctx_http::settings::{
    ContainerExecutionSettings, ContainerRuntimeKind, ExecutionMode, ExecutionSettings, Settings,
};
use serde_json::json;

#[tokio::test]
async fn workspace_attachments_are_db_canonical_and_ignore_repo_file() {
    let _sandbox_cli_available = common::TestEnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let _sandbox_cli_path =
        common::TestEnvGuard::set("CTX_HARNESS_SANDBOX_CLI_PATH", "/no/such/sandbox-cli");
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
    ctx_http::settings::save_settings(
        state.global_store(),
        &Settings {
            execution: Some(ExecutionSettings {
                mode: ExecutionMode::Host,
                container: ContainerExecutionSettings {
                    runtime: ContainerRuntimeKind::NativeContainer,
                    ..ContainerExecutionSettings::default()
                },
            }),
            ..Settings::default()
        },
    )
    .await
    .unwrap();
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
async fn workspace_attachments_reject_doc_mirror_local_script_sources() {
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let script_path = repo.path().join(".ctx").join("scripts").join("docs.py");
    tokio::fs::create_dir_all(script_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&script_path, "print('should not run')\n")
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

    let (create_status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        Some(json!({
            "kind": "doc_mirror",
            "name": "local-docs",
            "source": script_path.to_string_lossy().to_string()
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::BAD_REQUEST);
    let error = body["error"].as_str().unwrap_or_default();
    assert!(error.contains("http(s) URL"), "unexpected error: {error}");
    assert!(error.contains("not supported"), "unexpected error: {error}");
}

#[tokio::test]
async fn workspace_attachments_reject_doc_mirror_rw_mode() {
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;

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

    let (create_status, body): (StatusCode, serde_json::Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        Some(json!({
            "kind": "doc_mirror",
            "name": "docs",
            "source": "https://example.com/docs",
            "mode": "rw"
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::BAD_REQUEST);
    let error = body["error"].as_str().unwrap_or_default();
    assert!(error.contains("read-only"), "unexpected error: {error}");
}
