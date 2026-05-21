use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use ctx_core::models::{Workspace, Worktree};
use ctx_settings_model::{ContainerRuntimeKind, ExecutionMode};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;
use ctx_worktree_vcs_service::{
    SandboxContainerRuntime, VcsHooksHost, WorktreeExecutionLocation, WorktreeHookExecution,
};

use crate::daemon::execution_effective;
use crate::daemon::DaemonState;

#[path = "host/git_config.rs"]
mod git_config;

#[async_trait]
impl VcsHooksHost for DaemonState {
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
        git_config::sandbox_git_config_get(self, workspace, worktree, execution, key).await
    }

    async fn sandbox_git_config_set(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
        value: &str,
    ) -> Result<()> {
        git_config::sandbox_git_config_set(self, workspace, worktree, execution, key, value).await
    }

    async fn sandbox_git_config_unset(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
    ) -> Result<()> {
        git_config::sandbox_git_config_unset(self, workspace, worktree, execution, key).await
    }
}
