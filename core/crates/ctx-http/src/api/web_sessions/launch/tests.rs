use super::*;

use std::collections::HashMap;

use chrono::Utc;
use ctx_core::models::{ExecutionEnvironment, VcsKind, Worktree};
use ctx_settings_service::{CTX_HOST_EXECUTION_POLICY_ENV, EXECUTION_POLICY_TEST_ENV_LOCK};
use ctx_store::StoreManager;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
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

async fn test_state(data_root: &std::path::Path) -> Arc<AppState> {
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ))
}

fn sample_worktree(workspace_id: ctx_core::ids::WorkspaceId, root_path: PathBuf) -> Worktree {
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

fn assert_launch_error(
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

#[tokio::test]
async fn create_web_session_rejects_sandbox_only_before_host_runtime_setup() {
    let _env_lock = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
    let _policy = EnvVarGuard::set(CTX_HOST_EXECUTION_POLICY_ENV, "sandbox_only");
    let data_root = tempfile::tempdir().expect("tempdir");
    let state = test_state(data_root.path()).await;

    let err = create_web_session(
        &state,
        WebSessionLaunchRequest {
            session_id: None,
            worktree_id: None,
            url: "https://example.com".to_string(),
            viewport: None,
            fps: None,
        },
    )
    .await
    .expect_err("sandbox-only policy should reject host web sessions");

    assert_launch_error(
        &err,
        WebSessionLaunchErrorKind::Forbidden,
        "web sessions currently run on the host",
    );
    assert!(err.message().contains("host execution is disabled"));
}

#[tokio::test]
async fn create_web_session_rejects_sandbox_session_before_host_runtime_setup() {
    let _env_lock = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
    let _policy = EnvVarGuard::set(CTX_HOST_EXECUTION_POLICY_ENV, "allow_host");
    let data_root = tempfile::tempdir().expect("tempdir");
    let state = test_state(data_root.path()).await;
    let workspace_root = data_root.path().join("workspace");
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
    let worktree = store
        .insert_worktree(sample_worktree(
            workspace.id,
            workspace_root.join("worktree"),
        ))
        .await
        .expect("insert worktree");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Sandbox,
            "codex".to_string(),
            "gpt-5.4".to_string(),
            "primary".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create session");
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .expect("index session");

    let err = create_web_session(
        &state,
        WebSessionLaunchRequest {
            session_id: Some(session.id),
            worktree_id: None,
            url: "https://example.com".to_string(),
            viewport: None,
            fps: None,
        },
    )
    .await
    .expect_err("sandbox session should reject host web sessions");

    assert_launch_error(
        &err,
        WebSessionLaunchErrorKind::Forbidden,
        "disabled for sandbox sessions",
    );
}

#[tokio::test]
async fn create_web_session_rejects_unscoped_host_launches() {
    let _env_lock = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
    let _policy = EnvVarGuard::set(CTX_HOST_EXECUTION_POLICY_ENV, "allow_host");
    let data_root = tempfile::tempdir().expect("tempdir");
    let state = test_state(data_root.path()).await;

    let err = create_web_session(
        &state,
        WebSessionLaunchRequest {
            session_id: None,
            worktree_id: None,
            url: "https://example.com".to_string(),
            viewport: None,
            fps: None,
        },
    )
    .await
    .expect_err("unscoped web sessions should be rejected");

    assert_launch_error(
        &err,
        WebSessionLaunchErrorKind::BadRequest,
        "session_id or worktree_id",
    );
}
