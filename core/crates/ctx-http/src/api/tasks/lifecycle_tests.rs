use super::*;
use crate::daemon::AppState;
use ctx_core::models::{SandboxGuestIdentity, SandboxSubstrate, VcsKind};
use ctx_store::{Store, StoreManager};
use std::collections::HashMap;

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

#[tokio::test]
async fn archive_task_reclaims_managed_worktree_but_preserves_rematerialization_state() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
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
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let host_materialization_root = temp.path().join("host-shadow");
    std::fs::create_dir_all(&host_materialization_root).expect("create host shadow root");
    store
        .upsert_sandbox_binding(SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace.id),
            substrate: SandboxSubstrate::SharedVmContainer,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            live_worktree_root: ctx_sandbox_contract::container_worktree_root(worktree.id)
                .to_string_lossy()
                .to_string(),
            execution_settings_json: None,
            container_name: Some(ctx_workspace_container::workspace_container_name(
                workspace.id,
            )),
            host_materialization_root: Some(
                host_materialization_root.to_string_lossy().to_string(),
            ),
            created_at: Utc::now(),
        })
        .await
        .expect("insert sandbox binding");

    let log_path = temp.path().join("sandbox-cli.log");
    let sandbox_cli_path = crate::test_support::write_running_container_sandbox_cli_shim(
        temp.path(),
        &log_path,
        &ctx_workspace_container::workspace_container_name(workspace.id),
    );
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let Json(_) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");
    let archived_task = store
        .get_task(task.id)
        .await
        .expect("load archived task")
        .expect("archived task exists");
    assert!(
        archived_task.archived_at.is_some(),
        "task should be archived after archive_task"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "archive should reclaim the canonical managed worktree root"
    );
    assert!(
        !branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "archive should reclaim the task worktree branch"
    );
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_some(),
        "archive should preserve the worktree row"
    );
    assert!(
        store
            .get_sandbox_binding(worktree.id)
            .await
            .expect("load binding")
            .is_some(),
        "archive should preserve the sandbox binding row for rematerialization"
    );
    assert!(
        tokio::fs::metadata(&host_materialization_root)
            .await
            .is_err(),
        "archive should remove the sandbox host materialization root"
    );
}

#[tokio::test]
async fn archive_task_reports_cleanup_failure_when_branch_reclaim_fails() {
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
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");
    let branch = worktree
        .git_branch
        .as_deref()
        .expect("branch name")
        .to_string();
    let _branch_lock = create_branch_lock(&repo_root, &branch);

    let Json(response) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");

    assert!(
        response.cleanup_failed,
        "archive should report cleanup failure when branch reclaim fails"
    );
    assert!(
        response.task.archived_at.is_some(),
        "task should still be archived after reporting cleanup failure"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "archive should still reclaim the managed worktree root"
    );
    assert!(
        branch_exists(&repo_root, &branch)
            .await
            .expect("check branch"),
        "archive should report failure if the ctx branch could not be reclaimed"
    );
}
