use super::infer_terminal_worktree;
use crate::daemon::AppState;
use chrono::Utc;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{VcsKind, Workspace, Worktree};
use ctx_sandbox_contract::{container_worktree_root, sandbox_worktree_root};
use ctx_store::StoreManager;
use ctx_transport_runtime::terminal_launch::TerminalLaunchErrorKind;
use ctx_workspace_services::worktree_vcs::managed_worktree_path;
use std::path::PathBuf;
use std::sync::Arc;

fn sample_workspace(root_path: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(uuid::Uuid::new_v4()),
        root_path: root_path.to_string(),
        name: "sample".to_string(),
        created_at: Utc::now(),
        vcs_kind: Some(VcsKind::Git),
    }
}

fn sample_worktree(workspace: &Workspace, root_path: PathBuf) -> Worktree {
    Worktree {
        id: WorktreeId(uuid::Uuid::new_v4()),
        workspace_id: workspace.id,
        root_path: root_path.to_string_lossy().to_string(),
        base_commit_sha: String::new(),
        git_branch: None,
        vcs_kind: workspace.vcs_kind.clone(),
        base_revision: None,
        vcs_ref: None,
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
    }
}

async fn test_state(data_root: &std::path::Path) -> Arc<AppState> {
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        std::collections::HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ))
}

#[test]
fn sandbox_worktree_root_maps_managed_host_worktree_to_container_root() {
    let data_root = tempfile::tempdir().unwrap();
    let workspace = sample_workspace("/host/ws");
    let worktree_id = WorktreeId(uuid::Uuid::new_v4());
    let managed_root = managed_worktree_path(data_root.path(), workspace.id, worktree_id);
    let mut worktree = sample_worktree(&workspace, managed_root);
    worktree.id = worktree_id;

    assert_eq!(
        sandbox_worktree_root(&workspace, &worktree),
        container_worktree_root(worktree_id)
    );
}

#[tokio::test]
async fn infer_terminal_worktree_returns_not_found_for_unknown_session_without_fallback() {
    let data_root = tempfile::tempdir().expect("tempdir");
    let state = test_state(data_root.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            data_root
                .path()
                .join("workspace")
                .to_string_lossy()
                .to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let _worktree = store
        .insert_worktree(sample_worktree(
            &workspace,
            data_root.path().join("workspace").join("wt-existing"),
        ))
        .await
        .expect("insert worktree");

    let err = infer_terminal_worktree(
        &state,
        workspace.id,
        Some(SessionId(uuid::Uuid::new_v4())),
        None,
    )
    .await
    .expect_err("unknown explicit session target should 404");

    assert_eq!(err.kind(), TerminalLaunchErrorKind::NotFound);
}

#[tokio::test]
async fn infer_terminal_worktree_returns_not_found_for_unknown_task_without_fallback() {
    let data_root = tempfile::tempdir().expect("tempdir");
    let state = test_state(data_root.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            data_root
                .path()
                .join("workspace")
                .to_string_lossy()
                .to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let _worktree = store
        .insert_worktree(sample_worktree(
            &workspace,
            data_root.path().join("workspace").join("wt-existing"),
        ))
        .await
        .expect("insert worktree");

    let err = infer_terminal_worktree(
        &state,
        workspace.id,
        None,
        Some(TaskId(uuid::Uuid::new_v4())),
    )
    .await
    .expect_err("unknown explicit task target should 404");

    assert_eq!(err.kind(), TerminalLaunchErrorKind::NotFound);
}
