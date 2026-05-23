use chrono::Utc;
use serde::Serialize;

use super::common::{
    file_completions_route_error, route_file_download_error, workspace_delete_route_error,
    workspace_harness_container_ensure_error, workspace_harness_container_status_error,
    workspace_hydration_route_error,
};
use crate::daemon::workspaces::{WorkspaceHarnessContainerError, WorkspaceHydrationError};
use crate::test_support::TestDaemon;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, VcsKind, Workspace, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, WorkspaceAttachment, WorkspaceAttachmentKind,
    WorkspaceAttachmentStatus, Worktree, WorktreeBootstrapStatus,
};
use ctx_route_contracts::workspaces::{
    UpdateAgentSystemPromptConfigRouteRequest, UpdateWorktreeBootstrapConfigRequest,
    WorkspaceActiveHeadBatchRouteResponse, WorkspaceActiveSnapshotRouteResponse,
    WorkspaceAttachmentRouteResponse, WorkspaceFileCompletionsRouteQuery,
    WorkspacePromptConfigRouteParams, WorkspaceRouteErrorKind, WorkspaceRouteParams,
    WorkspaceRouteResponse, WorktreeRouteParams, WorktreeRouteResponse,
};
use ctx_store::WorktreeBootstrapResultUpdate;

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

async fn create_route_contract_workspace(daemon: &TestDaemon, name: &str) -> Workspace {
    daemon
        .global_store()
        .create_workspace(
            name.to_string(),
            daemon.data_root().join(name).to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace")
}

async fn create_route_contract_workspace_with_store(daemon: &TestDaemon, name: &str) -> Workspace {
    let root = daemon.data_root().join(name);
    std::fs::create_dir_all(&root).expect("create workspace root");
    daemon
        .seed_workspace_for_test(name, &root, VcsKind::Git)
        .await
        .expect("seed workspace")
}

async fn create_route_contract_worktree(
    daemon: &TestDaemon,
    workspace: &Workspace,
    name: &str,
    bootstrap_log_path: Option<String>,
) -> Worktree {
    let root = daemon.data_root().join(name);
    std::fs::create_dir_all(&root).expect("create worktree root");
    let store = daemon
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            root.to_string_lossy().to_string(),
            "base-sha".to_string(),
            Some("main".to_string()),
        )
        .await
        .expect("create worktree");
    daemon
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("index worktree");
    if bootstrap_log_path.is_some() {
        let now = Utc::now();
        store
            .update_worktree_bootstrap_result(WorktreeBootstrapResultUpdate {
                worktree_id: worktree.id,
                status: WorktreeBootstrapStatus::Success,
                started_at: now,
                finished_at: now,
                exit_code: Some(0),
                timeout_sec: Some(30),
                error: None,
                log_path: bootstrap_log_path,
                log_truncated: Some(false),
                command: Some("true".to_string()),
                script_path: None,
            })
            .await
            .expect("update bootstrap result");
    }
    store
        .get_worktree(worktree.id)
        .await
        .expect("load worktree")
        .expect("worktree exists")
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

#[tokio::test]
async fn worktree_routes_reject_invalid_worktree_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspace_worktree();

    let get_error = handle
        .get_worktree_for_route_params(WorktreeRouteParams::new("not-a-worktree"))
        .await
        .unwrap_err();
    assert_eq!(get_error.kind(), WorkspaceRouteErrorKind::BadRequest);

    let log_error = handle
        .download_worktree_bootstrap_logs_for_route_params(WorktreeRouteParams::new(
            "not-a-worktree",
        ))
        .await
        .unwrap_err();
    assert_eq!(log_error.kind(), WorkspaceRouteErrorKind::BadRequest);
}

#[tokio::test]
async fn worktree_routes_map_missing_worktree_to_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspace_worktree();
    let missing = WorktreeId::new().0.to_string();

    let get_error = handle
        .get_worktree_for_route_params(WorktreeRouteParams::new(missing.clone()))
        .await
        .unwrap_err();
    assert_eq!(get_error.kind(), WorkspaceRouteErrorKind::NotFound);

    let log_error = handle
        .download_worktree_bootstrap_logs_for_route_params(WorktreeRouteParams::new(missing))
        .await
        .unwrap_err();
    assert_eq!(log_error.kind(), WorkspaceRouteErrorKind::NotFound);
}

#[tokio::test]
async fn worktree_bootstrap_logs_route_rejects_missing_blank_and_outside_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let workspace = create_route_contract_workspace_with_store(&daemon, "worktree-logs").await;
    let missing_path =
        create_route_contract_worktree(&daemon, &workspace, "missing-log", None).await;
    let blank_path =
        create_route_contract_worktree(&daemon, &workspace, "blank-log", Some(" ".to_string()))
            .await;
    let outside_path = temp.path().join("outside-bootstrap.log");
    std::fs::write(&outside_path, "outside").expect("write outside log");
    let log_root = ctx_observability::logs::logs_dir(daemon.data_root()).join("worktree-bootstrap");
    std::fs::create_dir_all(&log_root).expect("create bootstrap log root");
    let outside_path = create_route_contract_worktree(
        &daemon,
        &workspace,
        "outside-log",
        Some(outside_path.to_string_lossy().to_string()),
    )
    .await;
    let handle = daemon.handle().workspace_worktree();

    for worktree in [missing_path, blank_path, outside_path] {
        let error = handle
            .download_worktree_bootstrap_logs_for_route_params(WorktreeRouteParams::new(
                worktree.id.0.to_string(),
            ))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), WorkspaceRouteErrorKind::NotFound);
    }
}

#[tokio::test]
async fn harness_container_routes_reject_invalid_workspace_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspace_harness_container();

    let status_error = handle
        .workspace_harness_container_status_for_route_params(WorkspaceRouteParams::new(
            "not-a-workspace",
        ))
        .await
        .unwrap_err();
    assert_eq!(status_error.kind(), WorkspaceRouteErrorKind::BadRequest);

    let stop_error = handle
        .stop_workspace_harness_container_for_route(WorkspaceRouteParams::new("not-a-workspace"))
        .await
        .unwrap_err();
    assert_eq!(stop_error.kind(), WorkspaceRouteErrorKind::BadRequest);

    let ensure_error = handle
        .ensure_workspace_harness_container_for_route(WorkspaceRouteParams::new("not-a-workspace"))
        .await
        .unwrap_err();
    assert_eq!(ensure_error.kind(), WorkspaceRouteErrorKind::BadRequest);
}

#[tokio::test]
async fn harness_container_routes_map_missing_workspace_to_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspace_harness_container();
    let missing = WorkspaceId::new().0.to_string();

    let status_error = handle
        .workspace_harness_container_status_for_route_params(WorkspaceRouteParams::new(
            missing.clone(),
        ))
        .await
        .unwrap_err();
    assert_eq!(status_error.kind(), WorkspaceRouteErrorKind::NotFound);

    let stop_error = handle
        .stop_workspace_harness_container_for_route(WorkspaceRouteParams::new(missing.clone()))
        .await
        .unwrap_err();
    assert_eq!(stop_error.kind(), WorkspaceRouteErrorKind::NotFound);

    let ensure_error = handle
        .ensure_workspace_harness_container_for_route(WorkspaceRouteParams::new(missing))
        .await
        .unwrap_err();
    assert_eq!(ensure_error.kind(), WorkspaceRouteErrorKind::NotFound);
}

#[tokio::test]
async fn harness_container_routes_are_hermetic_without_running_container() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let workspace = create_route_contract_workspace_with_store(&daemon, "no-container").await;
    let handle = daemon.handle().workspace_harness_container();
    let params = WorkspaceRouteParams::new(workspace.id.0.to_string());

    let status = handle
        .workspace_harness_container_status_for_route_params(params.clone())
        .await
        .expect("status route");
    assert!(status.is_none());

    let stop_error = handle
        .stop_workspace_harness_container_for_route(params)
        .await
        .unwrap_err();
    assert_eq!(stop_error.kind(), WorkspaceRouteErrorKind::NotFound);
}

#[tokio::test]
async fn ensure_harness_container_preserves_settings_error_mapping() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let workspace =
        create_route_contract_workspace_with_store(&daemon, "invalid-runtime-settings").await;
    daemon
        .seed_invalid_workspace_runtime_settings_document_for_test(workspace.id, "{ not json")
        .await
        .expect("seed invalid runtime settings");
    let handle = daemon.handle().workspace_harness_container();

    let error = handle
        .ensure_workspace_harness_container_for_route(WorkspaceRouteParams::new(
            workspace.id.0.to_string(),
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert!(
        error.message().contains("workspace runtime settings"),
        "unexpected error: {}",
        error.message()
    );
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
    let handle = daemon.handle().workspace_prompt_bootstrap_config();
    let error = handle
        .worktree_bootstrap_config_for_route_params(WorkspaceRouteParams::new("not-a-workspace"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "invalid workspace id");
}

#[tokio::test]
async fn prompt_config_route_params_reject_invalid_workspace_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let handle = daemon.handle().workspace_prompt_bootstrap_config();
    let error = handle
        .agent_system_prompt_config_for_route(WorkspacePromptConfigRouteParams::new(
            "not-a-workspace",
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "invalid workspace id");
}

#[tokio::test]
async fn prompt_bootstrap_config_routes_treat_deleting_workspace_as_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let workspace = create_route_contract_workspace(&daemon, "deleting-prompt-bootstrap").await;
    daemon.stores().begin_workspace_delete(workspace.id).await;
    let handle = daemon.handle().workspace_prompt_bootstrap_config();

    let bootstrap_error = handle
        .worktree_bootstrap_config_for_route_params(WorkspaceRouteParams::new(
            workspace.id.0.to_string(),
        ))
        .await
        .unwrap_err();
    assert_eq!(bootstrap_error.kind(), WorkspaceRouteErrorKind::NotFound);
    assert_eq!(bootstrap_error.message(), "workspace not found");

    let prompt_error = handle
        .agent_system_prompt_config_for_route(WorkspacePromptConfigRouteParams::new(
            workspace.id.0.to_string(),
        ))
        .await
        .unwrap_err();
    assert_eq!(prompt_error.kind(), WorkspaceRouteErrorKind::NotFound);
    assert_eq!(prompt_error.message(), "workspace not found");
    daemon.stores().finish_workspace_delete(workspace.id).await;
}

#[tokio::test]
async fn prompt_bootstrap_config_routes_map_unavailable_workspace_store_to_internal() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let workspace = create_route_contract_workspace(&daemon, "unavailable-prompt-bootstrap").await;
    daemon
        .cache_rehydration_make_workspace_store_unopenable_for_test(workspace.id)
        .await
        .expect("block workspace store");
    let handle = daemon.handle().workspace_prompt_bootstrap_config();

    let bootstrap_error = handle
        .worktree_bootstrap_config_for_route_params(WorkspaceRouteParams::new(
            workspace.id.0.to_string(),
        ))
        .await
        .unwrap_err();
    assert_eq!(bootstrap_error.kind(), WorkspaceRouteErrorKind::Internal);

    let prompt_error = handle
        .agent_system_prompt_config_for_route(WorkspacePromptConfigRouteParams::new(
            workspace.id.0.to_string(),
        ))
        .await
        .unwrap_err();
    assert_eq!(prompt_error.kind(), WorkspaceRouteErrorKind::Internal);
}

#[tokio::test]
async fn prompt_bootstrap_config_routes_preserve_malformed_runtime_settings_statuses() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");
    let workspace = create_route_contract_workspace(&daemon, "invalid-prompt-bootstrap").await;
    daemon
        .seed_invalid_workspace_runtime_settings_document_for_test(workspace.id, "{ not json")
        .await
        .expect("seed invalid runtime settings");
    let handle = daemon.handle().workspace_prompt_bootstrap_config();

    let bootstrap_get_error = handle
        .worktree_bootstrap_config_for_route_params(WorkspaceRouteParams::new(
            workspace.id.0.to_string(),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        bootstrap_get_error.kind(),
        WorkspaceRouteErrorKind::Internal
    );

    let bootstrap_post_error = handle
        .update_worktree_bootstrap_config_for_route_params(
            WorkspaceRouteParams::new(workspace.id.0.to_string()),
            UpdateWorktreeBootstrapConfigRequest {
                setup_command: Some("true".to_string()),
                timeout_sec: Some(30),
                wait_for_completion: Some(true),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        bootstrap_post_error.kind(),
        WorkspaceRouteErrorKind::BadRequest
    );

    let prompt_get_error = handle
        .agent_system_prompt_config_for_route(WorkspacePromptConfigRouteParams::new(
            workspace.id.0.to_string(),
        ))
        .await
        .unwrap_err();
    assert_eq!(prompt_get_error.kind(), WorkspaceRouteErrorKind::Internal);

    let prompt_post_error = handle
        .update_agent_system_prompt_config_for_route(
            WorkspacePromptConfigRouteParams::new(workspace.id.0.to_string()),
            UpdateAgentSystemPromptConfigRouteRequest {
                system_prompt_append: Some("prompt".to_string()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(prompt_post_error.kind(), WorkspaceRouteErrorKind::Internal);
}
