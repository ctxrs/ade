use super::*;
use chrono::Utc;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{
    SandboxBinding, SandboxGuestIdentity, SandboxProfile, SandboxSubstrate, VcsKind,
};
use ctx_store::StoreManager;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use ctx_daemon::daemon::{DaemonHandle, DaemonState};

#[tokio::test]
async fn get_worktree_returns_live_root_for_bound_sandbox_worktree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_root = temp.path().join("repo");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let state = Arc::new(DaemonState::new(
        temp.path().to_path_buf(),
        StoreManager::open(temp.path()).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let host_root = temp.path().join("managed-worktree");
    std::fs::create_dir_all(&host_root).expect("create managed worktree");
    let worktree = store
        .insert_worktree(Worktree {
            id: WorktreeId(Uuid::new_v4()),
            workspace_id: workspace.id,
            root_path: host_root.to_string_lossy().to_string(),
            base_commit_sha: "abc123".to_string(),
            git_branch: Some("ctx/test".to_string()),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some("abc123".to_string()),
            vcs_ref: Some("ctx/test".to_string()),
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        })
        .await
        .expect("insert worktree");
    store
        .upsert_sandbox_binding(SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace.id),
            substrate: SandboxSubstrate::SharedVmContainer,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: "/ctx/ws".to_string(),
            live_worktree_root: format!("/ctx/ws/worktrees/{}", worktree.id.0),
            execution_settings_json: None,
            container_name: Some("ctx-test".to_string()),
            host_materialization_root: None,
            created_at: Utc::now(),
        })
        .await
        .expect("upsert sandbox binding");
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");

    let Json(response) = get_worktree(
        State(DaemonHandle::new(state.clone()).workspaces()),
        Path(worktree.id.0.to_string()),
    )
    .await
    .expect("get worktree");

    assert_eq!(
        response.root_path,
        format!("/ctx/ws/worktrees/{}", worktree.id.0)
    );
    assert_eq!(response.id, worktree.id);
    assert_eq!(response.workspace_id, worktree.workspace_id);
    state.test_request_shutdown();
}

#[tokio::test]
async fn missing_worktree_routes_return_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = Arc::new(DaemonState::new(
        temp.path().to_path_buf(),
        StoreManager::open(temp.path()).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));
    let workspaces = State(DaemonHandle::new(state.clone()).workspaces());
    let missing_worktree_id = WorktreeId(Uuid::new_v4()).0.to_string();

    let status = get_worktree(workspaces.clone(), Path(missing_worktree_id.clone()))
        .await
        .expect_err("missing worktree should not resolve");
    assert_eq!(status, StatusCode::NOT_FOUND);

    let status = get_worktree_bootstrap_logs(workspaces, Path(missing_worktree_id))
        .await
        .expect_err("missing worktree bootstrap log should not resolve");
    assert_eq!(status, StatusCode::NOT_FOUND);

    state.test_request_shutdown();
}
