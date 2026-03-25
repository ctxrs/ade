use super::resolve_container_terminal_cwd;
use crate::disk_isolated;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::{sandbox_worktree_root, WorktreeDataPlane};
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

fn sandbox_data_plane(workspace: &Workspace, worktree: &Worktree) -> WorktreeDataPlane {
    WorktreeDataPlane {
        binding: None,
        workspace: workspace.clone(),
        execution_mode: ExecutionMode::Sandbox,
        live_workspace_root: PathBuf::from("/ctx/ws"),
        live_worktree_root: sandbox_worktree_root(workspace, worktree),
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
