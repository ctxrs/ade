use super::*;

pub(super) async fn remove_worktree(
    workspace_root: impl AsRef<StdPath>,
    worktree_path: impl AsRef<StdPath>,
) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path.as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree remove")?;
    if !output.status.success() {
        bail!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if tokio::fs::metadata(worktree_path.as_ref()).await.is_ok() {
        tokio::fs::remove_dir_all(worktree_path.as_ref())
            .await
            .context("removing worktree dir")?;
    }
    Ok(())
}

pub(super) async fn prune_worktrees(workspace_root: impl AsRef<StdPath>) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("prune")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree prune")?;
    if !output.status.success() {
        bail!(
            "git worktree prune failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

pub(crate) async fn ensure_worktree_attached(
    workspace_root: impl AsRef<StdPath>,
    worktree_path: impl AsRef<StdPath>,
    base_commit_sha: &str,
    branch_name: &str,
) -> anyhow::Result<()> {
    let workspace_root = workspace_root.as_ref();
    let worktree_path = worktree_path.as_ref();
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating worktree parent dir")?;
    }

    let mut prune_stale_registration = false;
    match tokio::fs::metadata(worktree_path).await {
        Ok(metadata) => {
            if is_git_worktree(worktree_path).await.unwrap_or(false) {
                return Ok(());
            }
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(worktree_path)
                    .await
                    .context("removing stale worktree dir")?;
            } else {
                tokio::fs::remove_file(worktree_path)
                    .await
                    .context("removing stale worktree file")?;
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            prune_stale_registration = true;
        }
        Err(err) => {
            return Err(err).context("reading managed worktree root metadata");
        }
    }

    if prune_stale_registration {
        prune_worktrees(workspace_root)
            .await
            .context("pruning stale managed worktree registrations")?;
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(workspace_root)
        .arg("worktree")
        .arg("add")
        .arg(worktree_path);
    if branch_exists(workspace_root, branch_name).await? {
        cmd.arg(branch_name);
    } else {
        cmd.arg("-b").arg(branch_name).arg(base_commit_sha);
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree add")?;
    if !output.status.success() {
        bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

pub(crate) async fn branch_exists(
    workspace_root: impl AsRef<StdPath>,
    branch_name: &str,
) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("show-ref")
        .arg("--verify")
        .arg("--quiet")
        .arg(format!("refs/heads/{branch_name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git show-ref --verify")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    bail!(
        "git show-ref failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

pub(super) async fn is_git_worktree(worktree_path: impl AsRef<StdPath>) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_path.as_ref())
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git rev-parse --is-inside-work-tree")?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}
