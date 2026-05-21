use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use ctx_core::models::{Workspace, Worktree};
use ctx_settings_model::{ContainerRuntimeKind, ExecutionMode};
use ctx_worktree_bootstrap_service::{
    bootstrap_command_env, run_bootstrap_command, shell_bootstrap_command, BootstrapCommandResult,
    BootstrapCommandRuntime, BootstrapStep,
};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use crate::daemon::execution_effective;
use crate::daemon::DaemonState;

struct SandboxBootstrapContext<'a> {
    settings: &'a ctx_settings_model::ExecutionSettings,
    live_workspace_root: &'a Path,
    live_worktree_root: &'a Path,
}

pub(super) async fn run_bootstrap_step(
    state: &DaemonState,
    step: &BootstrapStep,
    workspace: &Workspace,
    worktree: &Worktree,
    timeout: Duration,
) -> Result<BootstrapCommandResult> {
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

    let mut cmd = shell_bootstrap_command(&step.command);
    cmd.current_dir(&live_worktree_root);
    for (key, value) in bootstrap_command_env(worktree, &live_workspace_root, &live_worktree_root) {
        cmd.env(key, value);
    }
    run_bootstrap_command(cmd, timeout, BootstrapCommandRuntime::Host).await
}

async fn run_bootstrap_step_in_container(
    state: &DaemonState,
    step: &BootstrapStep,
    workspace: &Workspace,
    worktree: &Worktree,
    sandbox: SandboxBootstrapContext<'_>,
    timeout: Duration,
) -> Result<BootstrapCommandResult> {
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
    let env = bootstrap_command_env(
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

    run_bootstrap_command(cmd, timeout, BootstrapCommandRuntime::Container).await
}
