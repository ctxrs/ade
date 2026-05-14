use super::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use ctx_core::models::{VcsKind, Worktree};
use ctx_store::StoreManager;

pub(super) struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: Guarded by EXECUTION_POLICY_TEST_ENV_LOCK in every test using this helper.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            // SAFETY: Guarded by EXECUTION_POLICY_TEST_ENV_LOCK in every test using this helper.
            unsafe { std::env::set_var(self.key, previous) };
        } else {
            // SAFETY: Guarded by EXECUTION_POLICY_TEST_ENV_LOCK in every test using this helper.
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

pub(super) async fn test_state(data_root: &Path) -> Arc<DaemonState> {
    Arc::new(DaemonState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ))
}

pub(super) fn sample_worktree(
    workspace_id: ctx_core::ids::WorkspaceId,
    root_path: PathBuf,
) -> Worktree {
    Worktree {
        id: WorktreeId(uuid::Uuid::new_v4()),
        workspace_id,
        root_path: root_path.to_string_lossy().to_string(),
        base_commit_sha: String::new(),
        git_branch: None,
        vcs_kind: Some(VcsKind::Git),
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

pub(super) fn assert_launch_error(
    error: &WebSessionLaunchError,
    kind: WebSessionLaunchErrorKind,
    message_contains: &str,
) {
    assert_eq!(error.kind(), kind);
    assert!(
        error.message().contains(message_contains),
        "expected {:?} to contain {:?}",
        error.message(),
        message_contains
    );
}
