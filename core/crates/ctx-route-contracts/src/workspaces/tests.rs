use chrono::Utc;
use serde::Serialize;

use super::*;

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
fn workspace_stream_route_params_and_errors_preserve_status_contract() {
    let workspace = WorkspaceStreamRouteParams::new("not-a-workspace")
        .parse_workspace_id()
        .unwrap_err();
    assert_eq!(workspace.kind(), WorkspaceStreamRouteErrorKind::BadRequest);
    assert_eq!(workspace.message(), "invalid workspace id");

    let not_found = WorkspaceStreamRouteError::not_found("workspace not found");
    assert_eq!(not_found.kind(), WorkspaceStreamRouteErrorKind::NotFound);
    assert_eq!(not_found.message(), "workspace not found");

    let internal = WorkspaceStreamRouteError::internal("store failed");
    assert_eq!(internal.kind(), WorkspaceStreamRouteErrorKind::Internal);
    assert_eq!(internal.message(), "store failed");
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
fn workspace_management_route_dtos_preserve_wire_shape() {
    let create: CreateWorkspaceRequest = serde_json::from_value(serde_json::json!({
        "root_path": "/tmp/workspace",
        "name": "workspace"
    }))
    .expect("create request");
    assert_eq!(create.root_path, "/tmp/workspace");
    assert_eq!(create.name.as_deref(), Some("workspace"));

    let create_without_name: CreateWorkspaceRequest = serde_json::from_value(serde_json::json!({
        "root_path": "/tmp/workspace"
    }))
    .expect("create request without name");
    assert_eq!(create_without_name.name, None);

    let update: UpdateWorkspacePrimaryBranchRequest =
        serde_json::from_value(serde_json::json!({"primary_branch": "main"}))
            .expect("primary branch request");
    assert_eq!(update.primary_branch, "main");

    assert_same_json(
        WorkspacePrimaryBranchSnapshot {
            primary_branch: "main".to_string(),
        },
        serde_json::json!({"primary_branch": "main"}),
    );
    assert_same_json(
        WorkspaceConfigUpdateResult { ok: true },
        serde_json::json!({"ok": true}),
    );
}

#[test]
fn workspace_attachment_requests_preserve_validation_contracts() {
    let sync: SyncWorkspaceAttachmentsRouteRequest =
        serde_json::from_value(serde_json::json!({})).expect("sync request");
    assert!(!sync.refresh());
    let sync: SyncWorkspaceAttachmentsRouteRequest =
        serde_json::from_value(serde_json::json!({"refresh": true})).expect("sync request");
    assert!(sync.refresh());

    let create: CreateWorkspaceAttachmentRouteRequest = serde_json::from_value(serde_json::json!({
        "kind": "reference_repo",
        "name": "ref",
        "source": "https://example.test/repo.git",
        "revision": "main",
        "mount_relpath": "refs/ref",
        "mode": "ro",
        "update_policy": "manual"
    }))
    .expect("create request");
    let spec = create.into_spec().expect("valid spec");
    assert_eq!(spec.kind, WorkspaceAttachmentKind::ReferenceRepo);
    assert_eq!(spec.name, "ref");
    assert_eq!(spec.source, "https://example.test/repo.git");
    assert_eq!(spec.revision.as_deref(), Some("main"));
    assert_eq!(spec.subpath, None);
    assert_eq!(spec.mount_relpath.as_deref(), Some("refs/ref"));
    assert_eq!(spec.mode, Some(AttachmentMode::Ro));
    assert_eq!(spec.update_policy, Some(AttachmentUpdatePolicy::Manual));

    let error =
        serde_json::from_value::<CreateWorkspaceAttachmentRouteRequest>(serde_json::json!({
            "kind": "reference_repo",
            "name": " ",
            "source": "/tmp/ref"
        }))
        .expect("create request")
        .into_spec()
        .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "name and source are required");

    let delete: DeleteWorkspaceAttachmentRouteRequest = serde_json::from_value(serde_json::json!({
        "kind": "reference_repo",
        "name": "ref"
    }))
    .expect("delete request");
    let spec = delete.into_spec().expect("valid spec");
    assert_eq!(spec.kind, WorkspaceAttachmentKind::ReferenceRepo);
    assert_eq!(spec.name, "ref");

    let error = serde_json::from_value::<DeleteWorkspaceAttachmentRouteRequest>(
        serde_json::json!({"kind": "reference_repo", "name": ""}),
    )
    .expect("delete request")
    .into_spec()
    .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "name is required");
}
