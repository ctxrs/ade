use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use ctx_core::ids::TaskId;
use ctx_core::models::{Workspace, Worktree};
use ctx_harness_runtime::sandbox_container_command;
use ctx_workspace_container::workspace_container_name;
pub(crate) use ctx_workspace_services::vcs_hooks::cleanup_workspace_hooks;
use ctx_workspace_services::vcs_hooks::{
    SandboxContainerRuntime, VcsHooksHost, WorktreeExecutionLocation, WorktreeHookExecution,
};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use tokio::process::Command;

use crate::daemon::execution_effective;
use crate::daemon::AppState;
use ctx_settings_model::{ContainerRuntimeKind, ExecutionMode};
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

pub async fn ensure_task_commit_hook(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    task_id: TaskId,
) -> Result<()> {
    ctx_workspace_services::vcs_hooks::ensure_task_commit_hook(state, workspace, worktree, task_id)
        .await
}

pub async fn cleanup_worktree_hooks(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<()> {
    ctx_workspace_services::vcs_hooks::cleanup_worktree_hooks(state, workspace, worktree).await
}

#[async_trait]
impl VcsHooksHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    async fn worktree_execution(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Result<WorktreeHookExecution> {
        let data_plane = resolve_worktree_data_plane(self, worktree).await?;
        let settings =
            execution_effective::effective_execution_settings(self, workspace.id).await?;
        let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)?;
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(WorktreeHookExecution {
                location: WorktreeExecutionLocation::Host,
                live_worktree_root: None,
                container_runtime: None,
            });
        }
        Ok(WorktreeHookExecution {
            location: WorktreeExecutionLocation::Sandbox,
            live_worktree_root: Some(data_plane.live_worktree_root.to_string_lossy().to_string()),
            container_runtime: Some(match settings.container.runtime {
                ContainerRuntimeKind::NativeContainer => SandboxContainerRuntime::NativeContainer,
                ContainerRuntimeKind::SharedVmContainer => {
                    SandboxContainerRuntime::SharedVmContainer
                }
            }),
        })
    }

    async fn ensure_workspace_container(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Result<()> {
        let data_plane = resolve_worktree_data_plane(self, worktree).await?;
        let settings =
            execution_effective::effective_execution_settings(self, workspace.id).await?;
        let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)?;
        self.execution
            .harness
            .ensure_workspace_container_for_worktree(
                workspace,
                worktree,
                &settings,
                &self.core.daemon_url,
            )
            .await
    }

    async fn sandbox_git_config_get(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
    ) -> Result<Option<String>> {
        let mut cmd = sandbox_command(
            self,
            workspace,
            worktree,
            execution,
            "git",
            &[
                "config".to_string(),
                "--worktree".to_string(),
                "--get".to_string(),
                key.to_string(),
            ],
        )?;
        let output = cmd
            .output()
            .await
            .context("running sandbox git config --get")?;
        if output.status.success() {
            return Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            ));
        }
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        bail!(
            "sandbox git config --get failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }

    async fn sandbox_git_config_set(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let mut cmd = sandbox_command(
            self,
            workspace,
            worktree,
            execution,
            "git",
            &[
                "config".to_string(),
                "--worktree".to_string(),
                key.to_string(),
                value.to_string(),
            ],
        )?;
        let output = cmd.output().await.context("running sandbox git config")?;
        if !output.status.success() {
            bail!(
                "sandbox git config --worktree failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    async fn sandbox_git_config_unset(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
    ) -> Result<()> {
        let mut cmd = sandbox_command(
            self,
            workspace,
            worktree,
            execution,
            "git",
            &[
                "config".to_string(),
                "--worktree".to_string(),
                "--unset-all".to_string(),
                key.to_string(),
            ],
        )?;
        let output = cmd
            .output()
            .await
            .context("running sandbox git config --unset-all")?;
        if output.status.success() || matches!(output.status.code(), Some(1)) {
            return Ok(());
        }
        bail!(
            "sandbox git config --unset-all failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

fn sandbox_command(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    execution: &WorktreeHookExecution,
    command: &str,
    args: &[String],
) -> Result<Command> {
    let live_worktree_root = execution
        .live_worktree_root
        .as_ref()
        .context("sandbox hook execution missing live worktree root")?;
    let runtime = execution
        .container_runtime
        .context("sandbox hook execution missing runtime kind")?;
    match runtime {
        SandboxContainerRuntime::NativeContainer => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(live_worktree_root)
                .arg(workspace_container_name(workspace.id))
                .arg(command);
            cmd.args(args);
            Ok(cmd)
        }
        SandboxContainerRuntime::SharedVmContainer => {
            ctx_avf_linux_runtime::build_guest_exec_command(
                &state.core.data_root,
                workspace.id,
                worktree.id,
                Path::new(live_worktree_root),
                command,
                args,
                &HashMap::new(),
                None,
                false,
            )
        }
    }
}

#[cfg(test)]
mod tests;
