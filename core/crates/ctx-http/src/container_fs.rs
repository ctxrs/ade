use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ctx_core::ids::{WorkspaceId, WorktreeId};
use tokio::process::Command;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::ContainerRuntimeKind;
use crate::worktree_data_plane::{
    apply_data_plane_to_execution_settings, resolve_worktree_data_plane,
};

// Minimal container filesystem mediation for disk-isolated worktrees.
//
// v1 intentionally uses container CLI exec-based primitives (cat + stdin redirect) to avoid
// additional dependencies. This can be optimized later (tar streaming / container cp).

#[derive(Debug, Clone)]
enum ContainerFsBackend {
    NativeContainer {
        container_id: String,
    },
    SharedVmContainer {
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ContainerFs {
    data_root: PathBuf,
    backend: ContainerFsBackend,
}

impl ContainerFs {
    pub(crate) fn new(data_root: PathBuf, container_id: String) -> Self {
        Self {
            data_root,
            backend: ContainerFsBackend::NativeContainer { container_id },
        }
    }

    pub(crate) fn avf_linux_vm(
        data_root: PathBuf,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
    ) -> Self {
        Self {
            data_root,
            backend: ContainerFsBackend::SharedVmContainer {
                workspace_id,
                worktree_id,
            },
        }
    }

    pub(crate) async fn for_worktree(
        state: &Arc<AppState>,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
    ) -> Result<Self> {
        let workspace = state
            .global_store()
            .get_workspace(workspace_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workspace not found for worktree"))?;
        let worktree = state
            .global_store()
            .get_worktree(worktree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree not found"))?;
        let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
        let effective =
            execution_effective::effective_execution_settings(state, workspace_id).await?;
        let effective = apply_data_plane_to_execution_settings(&effective, &data_plane);
        state
            .execution
            .harness
            .ensure_workspace_container_for_worktree(
                &workspace,
                &worktree,
                &effective,
                &state.core.daemon_url,
            )
            .await?;
        Ok(match effective.container.runtime {
            ContainerRuntimeKind::NativeContainer => Self::new(
                state.core.data_root.clone(),
                crate::harness_runtime::workspace_container_name(workspace_id),
            ),
            ContainerRuntimeKind::SharedVmContainer => {
                Self::avf_linux_vm(state.core.data_root.clone(), workspace_id, worktree_id)
            }
        })
    }

    fn command_failure_detail(output: &std::process::Output) -> String {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "unknown sandbox filesystem failure".to_string()
        }
    }

    pub(crate) async fn read_to_string(&self, path: &Path) -> Result<String> {
        const SANDBOX_FS_TIMEOUT: Duration = Duration::from_secs(60);
        let out = match &self.backend {
            ContainerFsBackend::NativeContainer { .. } => {
                let mut cmd = self.base_exec().await?;
                cmd.arg("cat").arg("--").arg(path);
                crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_FS_TIMEOUT)
                    .await
                    .context("sandbox exec cat")?
            }
            ContainerFsBackend::SharedVmContainer {
                workspace_id,
                worktree_id,
            } => crate::workspace_runtime::run_avf_linux_guest_exec_capture(
                &self.data_root,
                *workspace_id,
                *worktree_id,
                path.parent().unwrap_or(path),
                "cat",
                &["--".to_string(), path.to_string_lossy().to_string()],
                &HashMap::new(),
                None,
                false,
            )
            .await
            .context("AVF guest exec cat")?,
        };
        if !out.status.success() {
            anyhow::bail!(
                "sandbox read failed (status {}): {}",
                out.status,
                Self::command_failure_detail(&out)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub(crate) async fn write_string(&self, path: &Path, contents: &str) -> Result<()> {
        const SANDBOX_FS_TIMEOUT: Duration = Duration::from_secs(60);
        // Use a tiny shell wrapper so we can safely redirect stdin to the target path.
        // Note: `/bin/sh` is typically `dash` in Ubuntu images, so avoid `pipefail`.
        let script = "set -eu; cat > \"$1\"";
        let mut cmd = match &self.backend {
            ContainerFsBackend::NativeContainer { .. } => {
                let mut cmd = self.base_exec().await?;
                cmd.arg("sh").arg("-lc").arg(script).arg("--").arg(path);
                cmd
            }
            ContainerFsBackend::SharedVmContainer {
                workspace_id,
                worktree_id,
            } => crate::workspace_runtime::build_avf_linux_guest_exec_command(
                &self.data_root,
                *workspace_id,
                *worktree_id,
                path.parent().unwrap_or(path),
                "sh",
                &[
                    "-lc".to_string(),
                    script.to_string(),
                    "--".to_string(),
                    path.to_string_lossy().to_string(),
                ],
                &HashMap::new(),
                None,
                false,
            )?,
        };
        cmd.stdin(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().context("spawning sandbox write")?;
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(contents.as_bytes()).await?;
        }
        let out = tokio::time::timeout(SANDBOX_FS_TIMEOUT, child.wait_with_output())
            .await
            .context("sandbox write timed out")??;
        if !out.status.success() {
            anyhow::bail!(
                "sandbox write failed (status {}): {}",
                out.status,
                Self::command_failure_detail(&out)
            );
        }
        Ok(())
    }

    async fn base_exec(&self) -> Result<Command> {
        let mut cmd = crate::harness_runtime::sandbox_container_command(&self.data_root)?;
        let ContainerFsBackend::NativeContainer { container_id } = &self.backend else {
            anyhow::bail!("container exec requested for non-native-container filesystem backend");
        };
        cmd.arg("exec").arg("--interactive").arg(container_id);
        Ok(cmd)
    }
}
