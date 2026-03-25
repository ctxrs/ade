use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tempfile::TempDir;

use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;

/// Container path for a disk-isolated worktree root.
pub fn container_worktree_root(worktree_id: WorktreeId) -> PathBuf {
    PathBuf::from("/ctx/ws/worktrees").join(worktree_id.0.to_string())
}

pub async fn remove_live_worktree_root(
    data_root: &Path,
    workspace_id: WorkspaceId,
    live_worktree_root: &Path,
) -> Result<()> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let container_id = format!("ctx-harness-{}", workspace_id.0);
    let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(&container_id)
        .arg("rm")
        .arg("-rf")
        .arg("--")
        .arg(live_worktree_root);
    let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
        .await
        .context("sandbox exec rm -rf disk-isolated worktree")?;
    if !out.status.success() {
        anyhow::bail!(
            "failed to remove disk-isolated worktree root {} (status {}): {}",
            live_worktree_root.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn verify_container_git_repo(
    data_root: &Path,
    container_id: &str,
    worktree_root: &Path,
) -> Result<()> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(worktree_root)
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg("git rev-parse --is-inside-work-tree && git rev-parse HEAD >/dev/null");
    let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
        .await
        .context("sandbox exec git repo verification")?;
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

async fn resolve_git_dir(worktree_root: &Path) -> Result<PathBuf> {
    let dotgit = worktree_root.join(".git");
    let meta = tokio::fs::symlink_metadata(&dotgit)
        .await
        .with_context(|| format!("reading {}", dotgit.display()))?;
    if meta.is_dir() {
        return Ok(dotgit);
    }
    let txt = tokio::fs::read_to_string(&dotgit)
        .await
        .with_context(|| format!("reading {}", dotgit.display()))?;
    let line = txt
        .lines()
        .find(|value| value.trim_start().starts_with("gitdir:"))
        .ok_or_else(|| anyhow::anyhow!("invalid .git file: missing gitdir"))?;
    let raw = line.trim_start().trim_start_matches("gitdir:").trim();
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(worktree_root.join(path))
    }
}

async fn resolve_common_git_dir(git_dir: &Path) -> Result<PathBuf> {
    let commondir = git_dir.join("commondir");
    let meta = match tokio::fs::symlink_metadata(&commondir).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(git_dir.to_path_buf());
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", commondir.display())),
    };
    if !meta.is_file() {
        return Ok(git_dir.to_path_buf());
    }
    let raw = tokio::fs::read_to_string(&commondir)
        .await
        .with_context(|| format!("reading {}", commondir.display()))?;
    let path = PathBuf::from(raw.trim());
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(git_dir.join(path))
    }
}

#[cfg(unix)]
fn symlink_path(target: &Path, dest: &Path, _is_dir: bool) -> Result<()> {
    std::os::unix::fs::symlink(target, dest)?;
    Ok(())
}

#[cfg(windows)]
fn symlink_path(target: &Path, dest: &Path, is_dir: bool) -> Result<()> {
    if is_dir {
        std::os::windows::fs::symlink_dir(target, dest)?;
    } else {
        std::os::windows::fs::symlink_file(target, dest)?;
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let entry_path = entry.path();
        let dest = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry_path, &dest)?;
        } else if file_type.is_symlink() {
            if dest.exists() {
                let _ = std::fs::remove_file(&dest);
                let _ = std::fs::remove_dir_all(&dest);
            }
            let link_target = std::fs::read_link(&entry_path)?;
            let is_dir = std::fs::metadata(&entry_path)
                .map(|meta| meta.is_dir())
                .unwrap_or(false);
            symlink_path(&link_target, &dest, is_dir)?;
        } else if file_type.is_file() {
            std::fs::copy(&entry_path, &dest)?;
        }
    }
    Ok(())
}

async fn prepare_self_contained_copy_root(
    data_root: &Path,
    source_root: &Path,
) -> Result<(PathBuf, Option<TempDir>)> {
    let dotgit = source_root.join(".git");
    let dotgit_meta = match tokio::fs::symlink_metadata(&dotgit).await {
        Ok(meta) => meta,
        Err(_) => return Ok((source_root.to_path_buf(), None)),
    };
    if dotgit_meta.is_dir() {
        return Ok((source_root.to_path_buf(), None));
    }

    let git_dir = resolve_git_dir(source_root).await?;
    let common_git_dir = resolve_common_git_dir(&git_dir).await?;
    let staging_parent = data_root.join("disk-isolated").join("staging");
    tokio::fs::create_dir_all(&staging_parent)
        .await
        .with_context(|| format!("creating {}", staging_parent.display()))?;
    let staging = TempDir::new_in(&staging_parent)
        .with_context(|| format!("creating temp dir in {}", staging_parent.display()))?;
    let staging_root = staging.path().join("worktree");
    let source = source_root.to_path_buf();
    let git_dir_copy = git_dir.clone();
    let common_git_dir_copy = common_git_dir.clone();
    let staging_copy = staging_root.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        copy_dir_recursive(&source, &staging_copy)?;
        let staged_dotgit = staging_copy.join(".git");
        if staged_dotgit.exists() {
            if staged_dotgit.is_dir() {
                std::fs::remove_dir_all(&staged_dotgit)?;
            } else {
                std::fs::remove_file(&staged_dotgit)?;
            }
        }
        copy_dir_recursive(&common_git_dir_copy, &staged_dotgit)?;
        if git_dir_copy != common_git_dir_copy {
            copy_dir_recursive(&git_dir_copy, &staged_dotgit)?;
        }
        let commondir = staged_dotgit.join("commondir");
        if commondir.exists() {
            std::fs::remove_file(&commondir)?;
        }
        let gitdir = staged_dotgit.join("gitdir");
        if gitdir.exists() {
            std::fs::remove_file(&gitdir)?;
        }
        Ok(())
    })
    .await??;

    Ok((staging_root, Some(staging)))
}

pub async fn ensure_worktree_from_host_copy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<PathBuf> {
    const SANDBOX_CP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let container_id = format!("ctx-harness-{}", workspace_id.0);
    let dest_root = container_worktree_root(worktree_id);
    tracing::info!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        container_id = %container_id,
        dest_root = %dest_root.display(),
        "provisioning disk-isolated worktree from host copy"
    );
    let (copy_root, _staging_guard) =
        prepare_self_contained_copy_root(data_root, host_workspace_root)
            .await
            .with_context(|| {
                format!(
                    "preparing self-contained sandbox copy root from {}",
                    host_workspace_root.display()
                )
            })?;

    // 1) Create destination directory.
    {
        let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
        cmd.arg("exec")
            .arg("--interactive")
            .arg(&container_id)
            .arg("mkdir")
            .arg("-p")
            .arg("--")
            .arg(&dest_root);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
            .await
            .context("sandbox exec mkdir")?;
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
        // Prefer `container cp` so we don't depend on any host-side `tar` binary being present
        // (notably on some Windows setups).
        // `container cp` copies directory contents when the source path ends in `/.` (or `\\.` on
        // Windows). Use `Path::join` to avoid hard-coding separators.
        let host_src = copy_root.join(".").to_string_lossy().to_string();
        let container_dst = format!("{}:{}", container_id, dest_root.to_string_lossy());
        let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
        cmd.arg("cp").arg(host_src).arg(container_dst);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_CP_TIMEOUT)
            .await
            .context("container cp host -> disk-isolated worktree")?;
        if !out.status.success() {
            anyhow::bail!(
                "container cp failed (status {}): {}",
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
        let mut chmod = crate::harness_runtime::sandbox_container_command(data_root)?;
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
            crate::harness_runtime::command_output_with_timeout(chmod, SANDBOX_EXEC_TIMEOUT).await;
    }

    // 3) Create/reset the worktree branch at the base revision.
    {
        let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
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
        let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
            .await
            .context("sandbox exec git checkout")?;
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

pub async fn ensure_workspace_root_from_host_copy(
    data_root: &Path,
    workspace: &Workspace,
) -> Result<PathBuf> {
    const SANDBOX_CP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let container_id = format!("ctx-harness-{}", workspace.id.0);
    let dest_root = PathBuf::from(crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT);
    if verify_container_git_repo(data_root, &container_id, &dest_root)
        .await
        .is_ok()
    {
        return Ok(dest_root);
    }

    let host_workspace_root = Path::new(&workspace.root_path);
    let (copy_root, _staging_guard) =
        prepare_self_contained_copy_root(data_root, host_workspace_root)
            .await
            .with_context(|| {
                format!(
                    "preparing self-contained sandbox workspace copy root from {}",
                    host_workspace_root.display()
                )
            })?;

    {
        let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
        cmd.arg("exec")
            .arg("--interactive")
            .arg(&container_id)
            .arg("mkdir")
            .arg("-p")
            .arg("--")
            .arg(&dest_root);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
            .await
            .context("sandbox exec mkdir for workspace root")?;
        if !out.status.success() {
            anyhow::bail!(
                "failed to create disk-isolated workspace dir (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }

    {
        let host_src = copy_root.join(".").to_string_lossy().to_string();
        let container_dst = format!("{}:{}", container_id, dest_root.to_string_lossy());
        let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
        cmd.arg("cp").arg(host_src).arg(container_dst);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_CP_TIMEOUT)
            .await
            .context("container cp host -> disk-isolated workspace root")?;
        if !out.status.success() {
            anyhow::bail!(
                "container cp for workspace root failed (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }

        let mut chmod = crate::harness_runtime::sandbox_container_command(data_root)?;
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
            crate::harness_runtime::command_output_with_timeout(chmod, SANDBOX_EXEC_TIMEOUT).await;
    }

    verify_container_git_repo(data_root, &container_id, &dest_root)
        .await
        .context("verifying seeded disk-isolated workspace root")?;
    Ok(dest_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn git(args: &[&str], cwd: &Path) {
        let status = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("run git command");
        assert!(status.success(), "git command failed: {args:?}");
    }

    #[tokio::test]
    async fn prepare_self_contained_copy_root_makes_git_worktree_clone_standalone() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        git(&["init", "-b", "main"], &repo_root);
        git(&["config", "user.name", "Test User"], &repo_root);
        git(&["config", "user.email", "test@example.com"], &repo_root);
        std::fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
        git(&["add", "README.md"], &repo_root);
        git(&["commit", "-m", "initial"], &repo_root);

        let worktree_root = temp.path().join("worktree");
        git(
            &[
                "worktree",
                "add",
                "-b",
                "ctx/test-worktree",
                worktree_root.to_str().expect("worktree path"),
            ],
            &repo_root,
        );

        let (copy_root, _guard) = prepare_self_contained_copy_root(temp.path(), &worktree_root)
            .await
            .expect("prepare self-contained root");
        assert_ne!(copy_root, worktree_root);
        assert!(copy_root.join(".git").is_dir());
        assert!(!copy_root.join(".git").join("commondir").exists());
        assert!(!copy_root.join(".git").join("gitdir").exists());

        let output = StdCommand::new("git")
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .current_dir(&copy_root)
            .output()
            .expect("run git rev-parse");
        assert!(
            output.status.success(),
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
    }
}
