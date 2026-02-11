use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use ctx_core::ids::{WorkspaceId, WorktreeId};

/// Container path for a disk-isolated worktree root.
pub fn container_worktree_root(worktree_id: WorktreeId) -> PathBuf {
    PathBuf::from("/ctx/ws/worktrees").join(worktree_id.0.to_string())
}

async fn verify_container_git_repo(
    data_root: &Path,
    container_id: &str,
    worktree_root: &Path,
) -> Result<()> {
    const PODMAN_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let mut cmd = crate::harness_runtime::podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(worktree_root)
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg("git rev-parse --is-inside-work-tree && git rev-parse HEAD >/dev/null");
    let out = crate::harness_runtime::command_output_with_timeout(cmd, PODMAN_EXEC_TIMEOUT)
        .await
        .context("podman exec git repo verification")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "unknown error".to_string()
        };
        anyhow::bail!(
            "disk-isolated worktree verification failed (status {}): {}",
            out.status,
            detail
        );
    }
    Ok(())
}

pub async fn ensure_worktree_from_host_copy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<PathBuf> {
    const PODMAN_CP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
    const PODMAN_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let container_id = format!("ctx-harness-{}", workspace_id.0);
    let dest_root = container_worktree_root(worktree_id);
    tracing::info!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        container_id = %container_id,
        dest_root = %dest_root.display(),
        "provisioning disk-isolated worktree from host copy"
    );

    // 1) Create destination directory.
    {
        let mut cmd = crate::harness_runtime::podman_command(data_root)?;
        cmd.arg("exec")
            .arg("--interactive")
            .arg(&container_id)
            .arg("mkdir")
            .arg("-p")
            .arg("--")
            .arg(&dest_root);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, PODMAN_EXEC_TIMEOUT)
            .await
            .context("podman exec mkdir")?;
        if !out.status.success() {
            anyhow::bail!(
                "failed to create disk-isolated worktree dir (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }

    // 2) Copy host workspace contents into the container worktree root.
    //
    // This is intentionally a one-time copy for v1. The disk-isolated worktree becomes the
    // canonical filesystem for the workbench + agents.
    {
        // Prefer `podman cp` so we don't depend on any host-side `tar` binary being present
        // (notably on some Windows setups).
        // `podman cp` copies directory contents when the source path ends in `/.` (or `\\.` on
        // Windows). Use `Path::join` to avoid hard-coding separators.
        let host_src = host_workspace_root.join(".").to_string_lossy().to_string();
        let container_dst = format!("{}:{}", container_id, dest_root.to_string_lossy());
        let mut cmd = crate::harness_runtime::podman_command(data_root)?;
        cmd.arg("cp").arg(host_src).arg(container_dst);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, PODMAN_CP_TIMEOUT)
            .await
            .context("podman cp host -> disk-isolated worktree")?;
        if !out.status.success() {
            anyhow::bail!(
                "podman cp failed (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        tracing::debug!(
            workspace_id = %workspace_id.0,
            worktree_id = %worktree_id.0,
            "disk-isolated host copy completed"
        );

        // Best-effort: ensure files are writable for the execution user.
        let mut chmod = crate::harness_runtime::podman_command(data_root)?;
        chmod
            .arg("exec")
            .arg("--interactive")
            .arg("--workdir")
            .arg(&dest_root)
            .arg(&container_id)
            .arg("sh")
            .arg("-lc")
            .arg("chmod -R u+rwX . >/dev/null 2>&1 || true");
        let _ =
            crate::harness_runtime::command_output_with_timeout(chmod, PODMAN_EXEC_TIMEOUT).await;
    }

    // 3) Create/reset the worktree branch at the base revision.
    {
        let mut cmd = crate::harness_runtime::podman_command(data_root)?;
        cmd.arg("exec")
            .arg("--interactive")
            .arg("--workdir")
            .arg(&dest_root)
            .arg(&container_id)
            .arg("git")
            .arg("checkout")
            .arg("-B")
            .arg(branch_name)
            .arg(base_commit_sha);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, PODMAN_EXEC_TIMEOUT)
            .await
            .context("podman exec git checkout")?;
        if !out.status.success() {
            anyhow::bail!(
                "git checkout failed (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        tracing::debug!(
            workspace_id = %workspace_id.0,
            worktree_id = %worktree_id.0,
            base_commit_sha = %base_commit_sha,
            branch_name = %branch_name,
            "disk-isolated checkout completed"
        );
    }

    // 4) Verify repository integrity after copy + checkout. We fail fast here so callers never
    // register a worktree/session that cannot answer git/diff requests later.
    verify_container_git_repo(data_root, &container_id, &dest_root).await?;
    tracing::info!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        "disk-isolated worktree repo verification succeeded"
    );

    Ok(dest_root)
}
