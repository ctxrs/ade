use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::http::StatusCode;
use axum::Json;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use url::Url;

use crate::api::errors::ApiErrorResp;
use crate::api::shared::status_code_for_request_or_policy_error;
use crate::daemon::AppState;
use crate::execution_policy::{ExecutionPolicyDenied, HostExecutionPolicy};
use crate::settings::ExecutionMode;
use crate::web_sessions::{WebSessionCreateRequest, WebSessionInfo, WebSessionViewport};
use ctx_core::ids::{SessionId, WorktreeId};

pub(crate) struct WebSessionLaunchRequest {
    pub(crate) session_id: Option<SessionId>,
    pub(crate) worktree_id: Option<WorktreeId>,
    pub(crate) url: String,
    pub(crate) viewport: Option<WebSessionViewport>,
    pub(crate) fps: Option<u32>,
}

struct WebSessionLaunchContext {
    work_dir: Option<PathBuf>,
}

pub(crate) async fn create_web_session(
    state: &Arc<AppState>,
    request: WebSessionLaunchRequest,
) -> Result<WebSessionInfo, (StatusCode, Json<ApiErrorResp>)> {
    validate_web_session_url(&request.url).map_err(|e| bad_request(e.to_string()))?;

    let launch_context =
        resolve_web_session_launch_context(state, request.session_id, request.worktree_id)
            .await
            .map_err(request_or_policy_error)?;

    let node_runtime = crate::installer::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
        ctx_provider_install::install_state::InstallTarget::Host,
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare node runtime: {e}")))?;

    let worker_bundle = crate::web_sessions::ensure_worker_bundle_for_node_runtime(
        &state.core.data_root,
        &node_runtime,
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

    if session_id.is_none() && worktree_id.is_none() {
        anyhow::bail!("web session launches must be scoped to a session_id or worktree_id");
    }

    let mut session_worktree_id = None;
    if let Some(session_id) = session_id {
        let store = state.store_for_session(session_id).await?;
        let session = store
            .get_session(session_id)
            .await?
            .context("session not found")?;
        if matches!(session.execution_environment, ExecutionEnvironment::Sandbox) {
            return Err(ExecutionPolicyDenied::new(
                "web sessions currently run on the host and are disabled for sandbox sessions until web sessions run inside the sandbox",
            )
            .into());
        }
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
    if store.get_sandbox_binding(worktree.id).await?.is_some() {
        return Err(ExecutionPolicyDenied::new(
            "web sessions currently run on the host and are disabled for sandbox worktrees until web sessions run inside the sandbox",
        )
        .into());
    }

    let effective =
        crate::execution_effective::effective_execution_settings(state, worktree.workspace_id)
            .await
            .context("loading workspace execution settings for web session")?;
    if matches!(effective.mode, ExecutionMode::Sandbox) {
        return Err(ExecutionPolicyDenied::new(
            "web sessions currently run on the host and are disabled for sandbox workspaces until web sessions run inside the sandbox",
        )
        .into());
    }
    Ok(())
}

fn error_response(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn bad_request(error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    error_response(StatusCode::BAD_REQUEST, error)
}

fn request_or_policy_error(error: anyhow::Error) -> (StatusCode, Json<ApiErrorResp>) {
    let status = status_code_for_request_or_policy_error(&error);
    error_response(status, format!("{error:#}"))
}

fn internal_error(error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error)
}

fn validate_web_session_url(raw: &str) -> anyhow::Result<()> {
    let parsed = Url::parse(raw).context("url must be an absolute URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!("url must use http:// or https://");
    }
    if parsed.host_str().is_none() {
        anyhow::bail!("url must include host");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::execution_policy::{CTX_HOST_EXECUTION_POLICY_ENV, EXECUTION_POLICY_TEST_ENV_LOCK};
    use chrono::Utc;
    use ctx_core::models::{ExecutionEnvironment, VcsKind, Worktree};
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

        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert!(err
            .1
             .0
            .error
            .contains("web sessions currently run on the host"));
        assert!(err.1 .0.error.contains("host execution is disabled"));
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

        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert!(err.1 .0.error.contains("disabled for sandbox sessions"));
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

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1 .0.error.contains("session_id or worktree_id"));
    }
}
