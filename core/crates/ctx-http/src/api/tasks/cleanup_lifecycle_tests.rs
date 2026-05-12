use super::*;
use crate::daemon::AppState;
use ctx_core::models::VcsKind;
use ctx_store::{Store, StoreManager};
use ctx_workspace_services::worktree_vcs::{managed_worktree_path, standaloneize_worktree_git_dir};
use std::collections::HashMap;

fn git(args: &[&str], cwd: &StdPath) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn git_output(args: &[&str], cwd: &StdPath) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git output");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_git_workspace(root: &StdPath) -> String {
    git(&["init"], root);
    git(&["symbolic-ref", "HEAD", "refs/heads/main"], root);
    git(&["config", "user.email", "ctx@example.com"], root);
    git(&["config", "user.name", "Ctx Test"], root);
    std::fs::write(root.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], root);
    git(&["commit", "-m", "initial"], root);
    git_output(&["rev-parse", "HEAD"], root)
}

async fn test_state(data_root: &StdPath) -> Arc<AppState> {
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ))
}

async fn insert_managed_worktree(
    store: &Store,
    data_root: &StdPath,
    workspace: &Workspace,
    owner_task_id: TaskId,
    repo_root: &StdPath,
    base_commit: &str,
) -> (Worktree, PathBuf) {
    let worktree_id = WorktreeId::new();
    let managed_root = managed_worktree_path(data_root, workspace.id, worktree_id);
    let branch_name = format!("ctx/{}/{}", owner_task_id.0, worktree_id.0);
    git(
        &[
            "worktree",
            "add",
            "-b",
            &branch_name,
            managed_root.to_string_lossy().as_ref(),
            base_commit,
        ],
        repo_root,
    );
    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit.to_string(),
            git_branch: Some(branch_name),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit.to_string()),
            vcs_ref: Some("".to_string()),
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
    (worktree, managed_root)
}

#[tokio::test]
async fn delete_task_prunes_and_deletes_branch_for_standalone_managed_worktree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");

    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    let branch = worktree
        .git_branch
        .clone()
        .expect("managed worktree branch should exist");
    standaloneize_worktree_git_dir(&managed_root)
        .await
        .expect("standaloneize managed worktree");

    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "standalone managed worktree root should be removed on task delete"
    );
    assert_eq!(
        git_output(&["branch", "--list", &branch], &repo_root),
        "",
        "standalone managed worktree branch should be pruned and deleted"
    );
    assert!(
        !git_output(&["worktree", "list", "--porcelain"], &repo_root)
            .contains(managed_root.to_string_lossy().as_ref()),
        "standalone managed worktree should be pruned from git worktree metadata"
    );
}
