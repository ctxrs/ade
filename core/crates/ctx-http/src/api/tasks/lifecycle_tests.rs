use super::*;
use crate::daemon::AppState;
use ctx_core::models::{SandboxGuestIdentity, SandboxSubstrate, VcsKind};
use ctx_store::{Store, StoreManager};
use std::collections::HashMap;

#[path = "lifecycle_tests/archive.rs"]
mod archive;
#[path = "lifecycle_tests/delete.rs"]
mod delete;
#[path = "lifecycle_tests/delete_cleanup.rs"]
mod delete_cleanup;
#[path = "lifecycle_tests/unarchive.rs"]
mod unarchive;
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

fn create_branch_lock(root: &StdPath, branch: &str) -> PathBuf {
    let lock_path = root
        .join(".git")
        .join("refs")
        .join("heads")
        .join(format!("{branch}.lock"));
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).expect("create branch lock parent");
    }
    std::fs::write(&lock_path, "").expect("create branch lock");
    lock_path
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

async fn save_test_execution_settings(
    state: &Arc<AppState>,
    execution: ctx_settings_model::ExecutionSettings,
) {
    let settings = ctx_settings_model::Settings {
        execution: Some(execution),
        ..Default::default()
    };
    ctx_settings_service::save_settings(state.global_store(), &settings)
        .await
        .expect("save runtime settings");
}

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
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
