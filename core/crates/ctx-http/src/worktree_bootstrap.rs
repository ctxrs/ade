use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree, WorktreeBootstrapNotice, WorktreeBootstrapStatus};
use ctx_store::WorktreeBootstrapResultUpdate;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use ctx_workspace_config as workspace_config;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

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

        Ok(
            ctx_workspace_services::worktree_bootstrap::normalize_bootstrap_config(
                ctx_workspace_services::worktree_bootstrap::BootstrapConfigInput {
                    setup_command: cfg.setup_command,
                    timeout_sec: cfg.timeout_sec,
                    wait_for_completion: cfg.wait_for_completion,
                },
            ),
        )
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
        let (log, log_truncated) =
            ctx_workspace_services::worktree_bootstrap::prepare_bootstrap_log_for_storage(
                &report.raw_log,
            );
        let log_path = ctx_workspace_services::worktree_bootstrap::write_bootstrap_log(
            &self.core.data_root,
            worktree.id,
            &log,
        )
        .await
        .ok();

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

    cmd.current_dir(&live_worktree_root);
    for (key, value) in ctx_workspace_services::worktree_bootstrap::bootstrap_command_env(
        worktree,
        &live_workspace_root,
        &live_worktree_root,
    ) {
        cmd.env(key, value);
    }
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
    let env = ctx_workspace_services::worktree_bootstrap::bootstrap_command_env(
        worktree,
        sandbox.live_workspace_root,
        sandbox.live_worktree_root,
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
