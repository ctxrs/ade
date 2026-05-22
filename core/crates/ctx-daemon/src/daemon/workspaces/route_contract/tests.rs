use chrono::Utc;
use serde::Serialize;

use super::super::WorkspaceRouteErrorKind;
use super::common::{
    file_completions_route_error, route_file_download_error, workspace_delete_route_error,
    workspace_harness_container_ensure_error, workspace_harness_container_status_error,
    workspace_hydration_route_error,
};
use super::*;

use crate::daemon::workspaces::{WorkspaceHarnessContainerError, WorkspaceHydrationError};
use crate::test_support::TestDaemon;
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, VcsKind, Workspace, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, WorkspaceAttachment, WorkspaceAttachmentKind,
    WorkspaceAttachmentStatus, Worktree, WorktreeBootstrapStatus,
};

fn assert_same_json<T, U>(left: T, right: U)
where
    T: Serialize,
    U: Serialize,
{
    assert_eq!(
        serde_json::to_value(left).unwrap(),
        serde_json::to_value(right).unwrap()
    );
}

#[test]
fn workspace_route_response_matches_workspace_wire_shape() {
    let workspace = Workspace {
        id: ctx_core::ids::WorkspaceId::new(),
        name: "workspace".to_string(),
        root_path: "/tmp/workspace".to_string(),
        created_at: Utc::now(),
        vcs_kind: Some(VcsKind::Git),
    };
    assert_same_json(WorkspaceRouteResponse::from(workspace.clone()), workspace);
}

#[test]
fn worktree_route_response_matches_worktree_wire_shape() {
    let now = Utc::now();
    let worktree = Worktree {
        id: ctx_core::ids::WorktreeId::new(),
        workspace_id: ctx_core::ids::WorkspaceId::new(),
        root_path: "/tmp/workspace/wt".to_string(),
        base_commit_sha: "abc123".to_string(),
        git_branch: Some("feature".to_string()),
        vcs_kind: Some(VcsKind::Git),
        base_revision: Some("rev-a".to_string()),
        vcs_ref: Some("main".to_string()),
        created_at: now,
        bootstrap_status: Some(WorktreeBootstrapStatus::Success),
        bootstrap_started_at: Some(now),
        bootstrap_finished_at: Some(now),
        bootstrap_exit_code: Some(0),
        bootstrap_timeout_sec: Some(60),
        bootstrap_error: Some("none".to_string()),
        bootstrap_log_path: Some("/tmp/bootstrap.log".to_string()),
        bootstrap_log_truncated: Some(false),
        bootstrap_command: Some("true".to_string()),
        bootstrap_script_path: Some("/tmp/bootstrap.sh".to_string()),
    };
    assert_same_json(WorktreeRouteResponse::from(worktree.clone()), worktree);
}

#[test]
fn workspace_attachment_route_response_matches_attachment_wire_shape() {
    let now = Utc::now();
    let attachment = WorkspaceAttachment {
        id: ctx_core::ids::WorkspaceAttachmentId::new(),
        workspace_id: ctx_core::ids::WorkspaceId::new(),
        kind: WorkspaceAttachmentKind::ReferenceRepo,
        name: "ref".to_string(),
        source: "https://example.test/repo.git".to_string(),
        revision: Some("main".to_string()),
        subpath: None,
        mount_relpath: "refs/ref".to_string(),
        mode: AttachmentMode::Ro,
        update_policy: AttachmentUpdatePolicy::Manual,
        status: WorkspaceAttachmentStatus::Pending,
        last_sync_at: None,
        error_message: Some("waiting".to_string()),
        created_at: now,
        updated_at: now,
    };
    assert_same_json(
        WorkspaceAttachmentRouteResponse::from(attachment.clone()),
        attachment,
    );
}

#[test]
fn active_workspace_route_wrappers_match_active_wire_shape() {
    let workspace_id = ctx_core::ids::WorkspaceId::new();
    let snapshot = WorkspaceActiveSnapshot {
        workspace_id,
        snapshot_rev: 7,
        archived_rev: 3,
        active: ctx_core::models::WorkspaceActivePage {
            tasks: Vec::new(),
            total_count: 0,
        },
    };
    assert_same_json(
        WorkspaceActiveSnapshotRouteResponse::from(snapshot.clone()),
        snapshot,
    );

    let heads = WorkspaceActiveHeadBatch {
        workspace_id,
        snapshot_rev: 7,
        heads: Vec::new(),
    };
    assert_same_json(
        WorkspaceActiveHeadBatchRouteResponse::from(heads.clone()),
        heads,
    );
}

#[test]
fn workspace_route_params_parse_invalid_ids_to_route_errors() {
    let workspace = WorkspaceRouteParams::new("not-a-workspace")
        .parse_workspace_id()
        .unwrap_err();
    assert_eq!(workspace.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(workspace.message(), "invalid workspace id");

    let worktree = WorktreeRouteParams::new("not-a-worktree")
        .parse_worktree_id()
        .unwrap_err();
    assert_eq!(worktree.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(worktree.message(), "invalid worktree id");
}

#[test]
fn workspace_route_error_helpers_preserve_status_classes() {
    let hydration = workspace_hydration_route_error(WorkspaceHydrationError::NotFound);
    assert_eq!(hydration.kind(), WorkspaceRouteErrorKind::NotFound);
    assert_eq!(hydration.message(), "workspace not found");

    let deletion = workspace_delete_route_error(super::super::WorkspaceDeleteError::NotFound);
    assert_eq!(deletion.kind(), WorkspaceRouteErrorKind::NotFound);
    assert_eq!(deletion.message(), "workspace not found");

    let download = route_file_download_error(crate::daemon::RouteFileDownloadError::NotFound);
    assert_eq!(download.kind(), WorkspaceRouteErrorKind::NotFound);

    let harness_status = workspace_harness_container_status_error(
        WorkspaceHarnessContainerError::ExecutionSettings(
            ctx_settings_service::EffectiveExecutionSettingsError::Internal(anyhow::anyhow!(
                "settings failed"
            )),
        ),
    );
    assert_eq!(harness_status.kind(), WorkspaceRouteErrorKind::Internal);

    let harness_ensure = workspace_harness_container_ensure_error(
        WorkspaceHarnessContainerError::Ensure(anyhow::anyhow!("bad container request")),
    );
    assert_eq!(harness_ensure.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(harness_ensure.message(), "bad container request");
}

#[test]
fn workspace_file_completions_query_preserves_http_query_shape() {
    let empty: WorkspaceFileCompletionsRouteQuery =
        serde_json::from_value(serde_json::json!({})).expect("empty query shape");
    assert_eq!(empty, WorkspaceFileCompletionsRouteQuery::default());

    let populated: WorkspaceFileCompletionsRouteQuery = serde_json::from_value(serde_json::json!({
        "query": "src",
        "limit": 25,
    }))
    .expect("populated query shape");
    let (query, limit) = populated.into_parts();
    assert_eq!(query.as_deref(), Some("src"));
    assert_eq!(limit, Some(25));
}

#[test]
fn workspace_file_completion_storage_errors_map_to_507_class() {
    let error = super::super::FileCompletionsError::from_internal_error(
        "resolving data plane",
        anyhow::anyhow!("No space left on device"),
    );
    let route_error = file_completions_route_error(error);
    assert_eq!(
        route_error.kind(),
        WorkspaceRouteErrorKind::InsufficientStorage
    );
}

#[tokio::test]
async fn attachment_route_params_reject_invalid_workspace_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspaces();
    let error = handle
        .create_and_sync_workspace_attachment_for_route_params(
            WorkspaceRouteParams::new("not-a-workspace"),
            serde_json::from_value(serde_json::json!({
                "kind": "reference_repo",
                "name": "ref",
                "source": "/tmp/ref"
            }))
            .expect("attachment request"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "invalid workspace id");
}

#[tokio::test]
async fn management_config_route_params_reject_invalid_workspace_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspaces();
    let error = handle
        .workspace_merge_queue_config_for_route_params(WorkspaceRouteParams::new("not-a-workspace"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "invalid workspace id");
}

#[tokio::test]
async fn worktree_bootstrap_route_params_reject_invalid_workspace_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspaces();
    let error = handle
        .worktree_bootstrap_config_for_route_params(WorkspaceRouteParams::new("not-a-workspace"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "invalid workspace id");
}
