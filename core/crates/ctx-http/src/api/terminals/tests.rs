use super::{infer_terminal_worktree, resolve_container_terminal_cwd, resolve_terminal_host_root};
use crate::daemon::AppState;
use crate::disk_isolated;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::{sandbox_worktree_root, WorktreeDataPlane};
use chrono::Utc;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{VcsKind, Workspace, Worktree};
use ctx_store::StoreManager;
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

fn sandbox_data_plane(workspace: &Workspace, worktree: &Worktree) -> WorktreeDataPlane {
    WorktreeDataPlane {
        binding: None,
        workspace: workspace.clone(),
        execution_mode: ExecutionMode::Sandbox,
        live_workspace_root: PathBuf::from("/ctx/ws"),
        live_worktree_root: sandbox_worktree_root(workspace, worktree),
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
    let managed_root =
        ctx_fs::worktrees::managed_worktree_path(data_root.path(), workspace.id, worktree_id);
    let mut worktree = sample_worktree(&workspace, managed_root.clone());
    worktree.id = worktree_id;

    assert_eq!(
        sandbox_worktree_root(&workspace, &worktree),
        disk_isolated::container_worktree_root(worktree_id)
    );
}

#[test]
fn resolve_container_terminal_cwd_maps_host_subdir_into_managed_container_worktree() {
    let data_root = tempfile::tempdir().unwrap();
    let workspace = sample_workspace("/host/ws");
    let worktree_id = WorktreeId(uuid::Uuid::new_v4());
    let managed_root =
        ctx_fs::worktrees::managed_worktree_path(data_root.path(), workspace.id, worktree_id);
    let mut worktree = sample_worktree(&workspace, managed_root.clone());
    worktree.id = worktree_id;
    let requested = managed_root.join("src/bin");
    let data_plane = sandbox_data_plane(&workspace, &worktree);

    let cwd = resolve_container_terminal_cwd(
        &data_plane,
        &PathBuf::from(&workspace.root_path),
        Some(&managed_root),
        Some(&requested),
    )
    .unwrap();

    assert_eq!(
        cwd,
        disk_isolated::container_worktree_root(worktree_id).join("src/bin")
    );
}

#[test]
fn resolve_container_terminal_cwd_maps_workspace_root_paths_into_container_workspace_root() {
    let workspace = sample_workspace("/host/ws");
    let worktree = sample_worktree(&workspace, PathBuf::from("/host/ws"));
    let requested = PathBuf::from("/host/ws/subdir");
    let data_plane = sandbox_data_plane(&workspace, &worktree);

    let cwd = resolve_container_terminal_cwd(
        &data_plane,
        &PathBuf::from(&workspace.root_path),
        Some(&PathBuf::from(&worktree.root_path)),
        Some(&requested),
    )
    .unwrap();

    assert_eq!(cwd, PathBuf::from("/ctx/ws/subdir"));
}

#[test]
fn resolve_container_terminal_cwd_maps_plain_workspace_terminal_paths_without_worktree_root() {
    let workspace = sample_workspace("/host/ws");
    let worktree = sample_worktree(&workspace, PathBuf::from("/host/ws"));
    let requested = PathBuf::from("/host/ws/subdir");
    let data_plane = sandbox_data_plane(&workspace, &worktree);

    let cwd = resolve_container_terminal_cwd(
        &data_plane,
        &PathBuf::from(&workspace.root_path),
        None,
        Some(&requested),
    )
    .unwrap();

    assert_eq!(cwd, PathBuf::from("/ctx/ws/subdir"));
}

#[tokio::test]
async fn resolve_terminal_host_root_preserves_missing_path_for_container_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let missing = temp.path().join("missing-root");

    let resolved = resolve_terminal_host_root(&missing, true, "workspace root is unavailable")
        .await
        .expect("sandbox mode should not require host path materialization");

    assert_eq!(resolved, missing);
}

#[tokio::test]
async fn resolve_terminal_host_root_requires_existing_path_for_host_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let missing = temp.path().join("missing-root");

    let err = resolve_terminal_host_root(&missing, false, "workspace root is unavailable")
        .await
        .expect_err("host terminals should still require a materialized host path");

    assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(err.1 .0.error, "workspace root is unavailable");
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

    assert_eq!(err.0, axum::http::StatusCode::NOT_FOUND);
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

    assert_eq!(err.0, axum::http::StatusCode::NOT_FOUND);
}
