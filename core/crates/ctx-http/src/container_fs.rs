use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::Command;

// Minimal container filesystem mediation for disk-isolated worktrees.
//
// v1 intentionally uses `podman exec`-based primitives (cat + stdin redirect) to avoid additional
// dependencies. This can be optimized later (tar streaming / podman cp).

#[derive(Debug, Clone)]
pub(crate) struct ContainerFs {
    data_root: PathBuf,
    container_id: String,
}

impl ContainerFs {
    pub(crate) fn new(data_root: PathBuf, container_id: String) -> Self {
        Self {
            data_root,
            container_id,
        }
    }

    pub(crate) async fn read_to_string(&self, path: &Path) -> Result<String> {
        const PODMAN_FS_TIMEOUT: Duration = Duration::from_secs(60);
        let mut cmd = self.base_exec().await?;
        cmd.arg("cat").arg("--").arg(path);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, PODMAN_FS_TIMEOUT)
            .await
            .context("podman exec cat")?;
        if !out.status.success() {
            anyhow::bail!(
                "podman exec cat failed (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub(crate) async fn write_string(&self, path: &Path, contents: &str) -> Result<()> {
        const PODMAN_FS_TIMEOUT: Duration = Duration::from_secs(60);
        // Use a tiny shell wrapper so we can safely redirect stdin to the target path.
        // Note: `/bin/sh` is typically `dash` in Ubuntu images, so avoid `pipefail`.
        let script = "set -eu; cat > \"$1\"";
        let mut cmd = self.base_exec().await?;
        cmd.arg("sh").arg("-lc").arg(script).arg("--").arg(path);
        cmd.stdin(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().context("spawning podman exec write")?;
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(contents.as_bytes()).await?;
        }
        let out = tokio::time::timeout(PODMAN_FS_TIMEOUT, child.wait_with_output())
            .await
            .context("podman exec write timed out")??;
        if !out.status.success() {
            anyhow::bail!(
                "podman exec write failed (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    async fn base_exec(&self) -> Result<Command> {
        let mut cmd = crate::harness_runtime::podman_command(&self.data_root)?;
        cmd.arg("exec").arg("--interactive").arg(&self.container_id);
        Ok(cmd)
    }
}

pub(crate) fn is_container_path(path: &Path) -> bool {
    // Disk-isolated worktrees use a fixed in-container root.
    path.to_string_lossy().starts_with("/ctx/")
}
