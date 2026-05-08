use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree, WorktreeBootstrapNotice, WorktreeBootstrapStatus};
use ctx_store::WorktreeBootstrapResultUpdate;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::logs;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_workspace_config as workspace_config;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

struct SandboxBootstrapContext<'a> {
    settings: &'a crate::settings::ExecutionSettings,
    live_workspace_root: &'a Path,
    live_worktree_root: &'a Path,
}

pub async fn spawn_worktree_bootstrap(
    state: Arc<AppState>,
    workspace: Workspace,
    worktree: Worktree,
) -> Result<()> {
    ctx_workspace_services::worktree_bootstrap::spawn_worktree_bootstrap(state, workspace, worktree)
        .await
}

#[async_trait]
impl ctx_workspace_services::worktree_bootstrap::WorktreeBootstrapHost for AppState {
    async fn load_bootstrap_config(
        &self,
        workspace: &Workspace,
    ) -> Result<Option<ctx_workspace_services::worktree_bootstrap::BootstrapConfig>> {
        let store = self.store_for_workspace(workspace.id).await?;
        let Some(cfg) = workspace_config::load_worktree_bootstrap_config(&store).await? else {
            return Ok(None);
        };

        let command = cfg
            .setup_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let Some(command) = command else {
            return Ok(None);
        };

        let timeout_sec = cfg.timeout_sec.unwrap_or(60);
        let timeout_sec = if timeout_sec == 0 { 60 } else { timeout_sec };
        let wait_for_completion = cfg.wait_for_completion.unwrap_or(false);
        Ok(Some(
            ctx_workspace_services::worktree_bootstrap::BootstrapConfig {
                timeout: Duration::from_secs(timeout_sec),
                command,
                wait_for_completion,
            },
        ))
    }

    async fn execute_bootstrap_step(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        step: &ctx_workspace_services::worktree_bootstrap::BootstrapStep,
        timeout: Duration,
    ) -> Result<ctx_workspace_services::worktree_bootstrap::BootstrapCommandResult> {
        run_bootstrap_step(self, step, workspace, worktree, timeout).await
    }

    async fn persist_bootstrap_report(
        &self,
        workspace_id: ctx_core::ids::WorkspaceId,
        worktree: &Worktree,
        report: ctx_workspace_services::worktree_bootstrap::BootstrapReport,
    ) {
        let (log, log_truncated) = ctx_workspace_services::worktree_bootstrap::truncate_log(
            &logs::redact_sensitive(&report.raw_log),
        );
        let log_path = write_bootstrap_log(self, worktree.id, &log).await.ok();

        update_bootstrap_result(
            self,
            WorktreeBootstrapResultUpdate {
                worktree_id: worktree.id,
                status: report.status.clone(),
                started_at: report.started_at,
                finished_at: report.finished_at,
                exit_code: report.exit_code,
                timeout_sec: Some(report.timeout_sec),
                error: report.error.clone(),
                log_path: log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                log_truncated: Some(log_truncated),
                command: report.command.clone(),
                script_path: None,
            },
        )
        .await;

        if report.status != WorktreeBootstrapStatus::Success {
            let notice = WorktreeBootstrapNotice {
                worktree_id: worktree.id,
                worktree_root: worktree.root_path.clone(),
                status: report.status,
                started_at: report.started_at,
                finished_at: report.finished_at,
                exit_code: report.exit_code,
                timeout_sec: Some(report.timeout_sec),
                command: report.command,
                script_path: None,
                log_path: log_path.map(|p| p.to_string_lossy().to_string()),
                log_truncated: Some(log_truncated),
                error: report.error,
            };
            emit_failure_notice(self, workspace_id, notice).await;
        }
    }

    async fn register_bootstrap(&self, worktree_id: WorktreeId, wait_for_completion: bool) {
        self.register_worktree_bootstrap(worktree_id, wait_for_completion)
            .await;
    }

    async fn finish_bootstrap(&self, worktree_id: WorktreeId) {
        self.finish_worktree_bootstrap(worktree_id).await;
    }
}

async fn run_bootstrap_step(
    state: &AppState,
    step: &ctx_workspace_services::worktree_bootstrap::BootstrapStep,
    workspace: &Workspace,
    worktree: &Worktree,
    timeout: Duration,
) -> Result<ctx_workspace_services::worktree_bootstrap::BootstrapCommandResult> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let settings = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)?;
    let execution_mode = settings.mode.clone();
    let live_workspace_root = data_plane.live_workspace_root;
    let live_worktree_root = data_plane.live_worktree_root;
    if matches!(execution_mode, ExecutionMode::Sandbox) {
        return run_bootstrap_step_in_container(
            state,
            step,
            workspace,
            worktree,
            SandboxBootstrapContext {
                settings: &settings,
                live_workspace_root: &live_workspace_root,
                live_worktree_root: &live_worktree_root,
            },
            timeout,
        )
        .await;
    }

    let mut cmd =
        ctx_workspace_services::worktree_bootstrap::shell_bootstrap_command(&step.command);

    cmd.current_dir(&live_worktree_root)
        .env("CTX_WORKSPACE_ROOT", &live_workspace_root)
        .env("CTX_WORKTREE_ROOT", &live_worktree_root)
        .env("CTX_WORKTREE_ID", worktree.id.0.to_string())
        .env(
            "CTX_BRANCH_NAME",
            worktree
                .vcs_ref
                .clone()
                .or_else(|| worktree.git_branch.clone())
                .unwrap_or_default(),
        )
        .env(
            "CTX_BASE_REVISION",
            worktree
                .base_revision
                .as_deref()
                .unwrap_or(&worktree.base_commit_sha),
        )
        .env(
            "CTX_BASE_COMMIT_SHA",
            worktree
                .base_revision
                .as_deref()
                .unwrap_or(&worktree.base_commit_sha),
        );
    ctx_workspace_services::worktree_bootstrap::run_bootstrap_command(
        cmd,
        timeout,
        ctx_workspace_services::worktree_bootstrap::BootstrapCommandRuntime::Host,
    )
    .await
}

async fn run_bootstrap_step_in_container(
    state: &AppState,
    step: &ctx_workspace_services::worktree_bootstrap::BootstrapStep,
    workspace: &Workspace,
    worktree: &Worktree,
    sandbox: SandboxBootstrapContext<'_>,
    timeout: Duration,
) -> Result<ctx_workspace_services::worktree_bootstrap::BootstrapCommandResult> {
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            workspace,
            worktree,
            sandbox.settings,
            &state.core.daemon_url,
        )
        .await?;
    let mut env = std::collections::HashMap::new();
    env.insert(
        "CTX_WORKSPACE_ROOT".to_string(),
        sandbox.live_workspace_root.to_string_lossy().to_string(),
    );
    env.insert(
        "CTX_WORKTREE_ROOT".to_string(),
        sandbox.live_worktree_root.to_string_lossy().to_string(),
    );
    env.insert("CTX_WORKTREE_ID".to_string(), worktree.id.0.to_string());
    env.insert(
        "CTX_BRANCH_NAME".to_string(),
        worktree
            .vcs_ref
            .clone()
            .or_else(|| worktree.git_branch.clone())
            .unwrap_or_default(),
    );
    env.insert(
        "CTX_BASE_REVISION".to_string(),
        worktree
            .base_revision
            .as_deref()
            .unwrap_or(&worktree.base_commit_sha)
            .to_string(),
    );
    env.insert(
        "CTX_BASE_COMMIT_SHA".to_string(),
        worktree
            .base_revision
            .as_deref()
            .unwrap_or(&worktree.base_commit_sha)
            .to_string(),
    );

    let cmd = match sandbox.settings.container.runtime {
        ContainerRuntimeKind::NativeContainer => {
            let container_name = ctx_workspace_container::workspace_container_name(workspace.id);
            let mut cmd = ctx_harness_runtime::sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(sandbox.live_worktree_root);
            for (key, value) in &env {
                cmd.arg("--env").arg(format!("{key}={value}"));
            }
            cmd.arg(container_name)
                .arg("sh")
                .arg("-lc")
                .arg(&step.command);
            cmd
        }
        ContainerRuntimeKind::SharedVmContainer => ctx_avf_linux_runtime::build_guest_exec_command(
            &state.core.data_root,
            workspace.id,
            worktree.id,
            sandbox.live_worktree_root,
            "sh",
            &["-lc".to_string(), step.command.clone()],
            &env,
            None,
            false,
        )?,
    };

    ctx_workspace_services::worktree_bootstrap::run_bootstrap_command(
        cmd,
        timeout,
        ctx_workspace_services::worktree_bootstrap::BootstrapCommandRuntime::Container,
    )
    .await
}

async fn write_bootstrap_log(
    state: &AppState,
    worktree_id: WorktreeId,
    contents: &str,
) -> Result<PathBuf> {
    let dir = logs::logs_dir(&state.core.data_root).join("worktree-bootstrap");
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating bootstrap log dir")?;
    let path = dir.join(format!("worktree-bootstrap-{}.log", worktree_id.0));
    tokio::fs::write(&path, contents)
        .await
        .with_context(|| format!("writing bootstrap log to {}", path.display()))?;
    Ok(path)
}

async fn update_bootstrap_result(state: &AppState, update: WorktreeBootstrapResultUpdate) {
    let store = match state.store_for_worktree(update.worktree_id).await {
        Ok(store) => store,
        Err(_) => return,
    };
    let _ = store.update_worktree_bootstrap_result(update).await;
}

async fn emit_failure_notice(
    state: &AppState,
    workspace_id: ctx_core::ids::WorkspaceId,
    notice: WorktreeBootstrapNotice,
) {
    state
        .workspaces
        .workspace_active_snapshot
        .publish_worktree_bootstrap(workspace_id, notice)
        .await;
}
