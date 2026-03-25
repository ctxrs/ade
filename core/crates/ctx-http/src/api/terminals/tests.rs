use super::{resolve_container_terminal_cwd, sandbox_worktree_root};
use crate::disk_isolated;
use chrono::Utc;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{VcsKind, Workspace, Worktree};
use std::path::PathBuf;

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
        sandbox_worktree_root(data_root.path(), &workspace, &worktree),
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

    let cwd = resolve_container_terminal_cwd(
        data_root.path(),
        &workspace,
        Some(&worktree),
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
    let data_root = tempfile::tempdir().unwrap();
    let workspace = sample_workspace("/host/ws");
    let worktree = sample_worktree(&workspace, PathBuf::from("/host/ws"));
    let requested = PathBuf::from("/host/ws/subdir");

    let cwd = resolve_container_terminal_cwd(
        data_root.path(),
        &workspace,
        Some(&worktree),
        Some(&requested),
    )
    .unwrap();

    assert_eq!(cwd, PathBuf::from("/ctx/ws/subdir"));
}
