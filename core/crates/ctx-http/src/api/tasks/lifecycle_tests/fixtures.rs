use super::*;
use crate::daemon::DaemonState;
use ctx_core::models::{Task, VcsKind, Workspace, Worktree};
use ctx_store::{Store, StoreManager};
use std::collections::HashMap;

#[path = "fixtures/git.rs"]
mod git;

pub(super) use git::{create_branch_lock, git, init_git_workspace};

pub(super) struct ManagedTaskFixture {
    pub(super) repo_root: PathBuf,
    pub(super) state: Arc<DaemonState>,
    pub(super) workspace: Workspace,
    pub(super) store: Store,
    pub(super) task: Task,
    pub(super) worktree: Worktree,
    pub(super) managed_root: PathBuf,
}

pub(super) async fn create_managed_task_fixture(data_root: &StdPath) -> ManagedTaskFixture {
    let repo_root = data_root.join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(data_root).await;
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
        data_root,
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

    ManagedTaskFixture {
        repo_root,
        state,
        workspace,
        store,
        task,
        worktree,
        managed_root,
    }
}

pub(super) async fn test_state(data_root: &StdPath) -> Arc<DaemonState> {
    Arc::new(DaemonState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ))
}

pub(super) async fn save_test_execution_settings(
    state: &Arc<DaemonState>,
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

pub(super) struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
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

pub(super) async fn insert_managed_worktree(
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
