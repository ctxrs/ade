use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_transport_runtime::web_sessions::{
    ensure_worker_bundle, validate_web_session_host_session, validate_web_session_host_worktree,
    validate_web_session_launch_scope, validate_web_session_url, NodeRuntimeSpec,
    WebSessionCreateRequest, WebSessionInfo, WebSessionLaunchPolicyError,
    WebSessionLaunchPolicyErrorKind, WebSessionViewport,
};

use crate::daemon::AppState;
use ctx_core::ids::{SessionId, WorktreeId};
use ctx_settings_model::ExecutionMode;
use ctx_settings_service::HostExecutionPolicy;

pub(crate) struct WebSessionLaunchRequest {
    pub(crate) session_id: Option<SessionId>,
    pub(crate) worktree_id: Option<WorktreeId>,
    pub(crate) url: String,
    pub(crate) viewport: Option<WebSessionViewport>,
    pub(crate) fps: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebSessionLaunchErrorKind {
    BadRequest,
    Forbidden,
    Internal,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct WebSessionLaunchError {
    kind: WebSessionLaunchErrorKind,
    message: String,
}

impl WebSessionLaunchError {
    pub(crate) fn kind(&self) -> WebSessionLaunchErrorKind {
        self.kind
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

struct WebSessionLaunchContext {
    work_dir: Option<PathBuf>,
}

pub(crate) async fn create_web_session(
    state: &Arc<AppState>,
    request: WebSessionLaunchRequest,
) -> Result<WebSessionInfo, WebSessionLaunchError> {
    validate_web_session_url(&request.url).map_err(|e| bad_request(e.to_string()))?;

    let launch_context =
        resolve_web_session_launch_context(state, request.session_id, request.worktree_id)
            .await
            .map_err(request_or_policy_error)?;

    let node_runtime = crate::daemon::installer::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
        ctx_provider_install::install_state::InstallTarget::Host,
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare node runtime: {e}")))?;

    let worker_bundle = ensure_worker_bundle(
        &state.core.data_root,
        &NodeRuntimeSpec {
            node_bin: node_runtime.node_bin.clone(),
            npm_cli_js: node_runtime.npm_cli_js.clone(),
        },
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare web session worker: {e}")))?;

    let handle = state
        .transport
        .web_sessions
        .create(WebSessionCreateRequest {
            url: request.url,
            viewport: request.viewport,
            fps: request.fps,
            work_dir: launch_context.work_dir,
            session_id: request.session_id.map(|id| id.0.to_string()),
            worktree_id: request.worktree_id.map(|id| id.0.to_string()),
            node_bin: node_runtime.node_bin,
            worker_path: worker_bundle.worker_path,
            node_modules_path: worker_bundle.node_modules_path,
        })
        .await
        .map_err(|e| internal_error(format!("failed to create web session: {e}")))?;

    Ok(handle.snapshot().await)
}

async fn resolve_web_session_launch_context(
    state: &Arc<AppState>,
    session_id: Option<SessionId>,
    worktree_id: Option<WorktreeId>,
) -> anyhow::Result<WebSessionLaunchContext> {
    HostExecutionPolicy::current()?
        .validate_execution_environment(ExecutionEnvironment::Host)
        .context("web sessions currently run on the host")?;

    validate_web_session_launch_scope(session_id.is_some(), worktree_id.is_some())?;

    let mut session_worktree_id = None;
    if let Some(session_id) = session_id {
        let store = state.store_for_session(session_id).await?;
        let session = store
            .get_session(session_id)
            .await?
            .context("session not found")?;
        validate_web_session_host_session(session.execution_environment)?;
        let worktree = store
            .get_worktree(session.worktree_id)
            .await?
            .context("worktree not found")?;
        validate_web_session_worktree(state, &store, &worktree).await?;
        session_worktree_id = Some(session.worktree_id);
    }

    if let Some(worktree_id) = worktree_id {
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        validate_web_session_worktree(state, &store, &worktree).await?;
        return Ok(WebSessionLaunchContext {
            work_dir: Some(PathBuf::from(worktree.root_path)),
        });
    }

    if let Some(worktree_id) = session_worktree_id {
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(WebSessionLaunchContext {
            work_dir: Some(PathBuf::from(worktree.root_path)),
        });
    }
    Ok(WebSessionLaunchContext { work_dir: None })
}

async fn validate_web_session_worktree(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    worktree: &Worktree,
) -> anyhow::Result<()> {
    let has_sandbox_binding = store.get_sandbox_binding(worktree.id).await?.is_some();
    let effective = crate::daemon::execution_effective::effective_execution_settings(
        state,
        worktree.workspace_id,
    )
    .await
    .context("loading workspace execution settings for web session")?;
    validate_web_session_host_worktree(
        has_sandbox_binding,
        matches!(effective.mode, ExecutionMode::Sandbox),
    )?;
    Ok(())
}

fn launch_error(
    kind: WebSessionLaunchErrorKind,
    message: impl Into<String>,
) -> WebSessionLaunchError {
    WebSessionLaunchError {
        kind,
        message: message.into(),
    }
}

fn bad_request(error: impl Into<String>) -> WebSessionLaunchError {
    launch_error(WebSessionLaunchErrorKind::BadRequest, error)
}

fn request_or_policy_error(error: anyhow::Error) -> WebSessionLaunchError {
    let kind = if let Some(policy_error) = error.downcast_ref::<WebSessionLaunchPolicyError>() {
        match policy_error.kind() {
            WebSessionLaunchPolicyErrorKind::BadRequest => WebSessionLaunchErrorKind::BadRequest,
            WebSessionLaunchPolicyErrorKind::Forbidden => WebSessionLaunchErrorKind::Forbidden,
        }
    } else if ctx_settings_service::is_execution_policy_denial(&error) {
        WebSessionLaunchErrorKind::Forbidden
    } else {
        WebSessionLaunchErrorKind::BadRequest
    };
    launch_error(kind, format!("{error:#}"))
}

fn internal_error(error: impl Into<String>) -> WebSessionLaunchError {
    launch_error(WebSessionLaunchErrorKind::Internal, error)
}

#[cfg(test)]
mod tests {
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
}
