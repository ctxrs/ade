use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use ctx_core::ids::{WorkspaceId, WorktreeId};

use crate::git;

pub fn worktrees_root(data_root: impl AsRef<Path>) -> PathBuf {
    data_root.as_ref().join("worktrees")
}

pub fn managed_worktree_path(
    data_root: impl AsRef<Path>,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) -> PathBuf {
    worktrees_root(data_root)
        .join(workspace_id.0.to_string())
        .join(worktree_id.0.to_string())
}

pub async fn create_worktree(
    workspace_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("add")
        .arg(worktree_path.as_ref())
        .arg("-b")
        .arg(branch_name)
        .arg(base_commit_sha)
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

pub async fn remove_worktree(
    workspace_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
) -> Result<()> {
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

pub async fn prune_worktrees(workspace_root: impl AsRef<Path>) -> Result<()> {
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

pub async fn ensure_worktree_attached(
    workspace_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<()> {
    let worktree_path = worktree_path.as_ref();
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating worktree parent dir")?;
    }

    if tokio::fs::metadata(worktree_path).await.is_ok() {
        if is_git_worktree(worktree_path).await.unwrap_or(false) {
            return Ok(());
        }
        tokio::fs::remove_dir_all(worktree_path)
            .await
            .context("removing stale worktree dir")?;
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(workspace_root.as_ref())
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

async fn branch_exists(workspace_root: impl AsRef<Path>, branch_name: &str) -> Result<bool> {
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

async fn is_git_worktree(worktree_path: impl AsRef<Path>) -> Result<bool> {
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

pub async fn diff_worktree(
    worktree_path: impl AsRef<Path>,
    base_commit_sha: &str,
) -> Result<String> {
    let root = worktree_path.as_ref();
    let mut out = git::git_diff(root, base_commit_sha).await?;

    // `git diff <base>` does not include untracked files, but we want the UI to show newly created
    // files even before they are staged.
    let untracked = git::list_untracked_files(root).await.unwrap_or_default();
    for rel in untracked {
        // Avoid dumping huge blobs into the diff view.
        let abs = root.join(&rel);
        if let Ok(meta) = tokio::fs::metadata(&abs).await {
            const MAX_UNTRACKED_BYTES: u64 = 512 * 1024;
            if meta.len() > MAX_UNTRACKED_BYTES {
                out.push_str(&format!(
                    "\n# untracked: {} ({} bytes; omitted)\n",
                    rel,
                    meta.len()
                ));
                continue;
            }
        }

        if let Ok(patch) = git::git_diff_untracked_file(root, &rel).await {
            if !patch.trim().is_empty() {
                out.push('\n');
                out.push_str(&patch);
            }
        }
    }

    Ok(out)
}

pub async fn diff_worktree_summary(
    worktree_path: impl AsRef<Path>,
    base_commit_sha: &str,
) -> Result<(i64, i64, i64)> {
    let root = worktree_path.as_ref();
    let mut file_count: i64 = 0;
    let mut additions: i64 = 0;
    let mut deletions: i64 = 0;

    let numstats = git::git_diff_numstat(root, base_commit_sha).await?;
    for (add, del, _path) in numstats {
        file_count += 1;
        additions += add;
        deletions += del;
    }

    let untracked = git::list_untracked_files(root).await.unwrap_or_default();
    if !untracked.is_empty() {
        for rel in untracked {
            file_count += 1;
            let abs = root.join(&rel);
            if let Ok(meta) = tokio::fs::metadata(&abs).await {
                const MAX_UNTRACKED_BYTES: u64 = 512 * 1024;
                if meta.len() > MAX_UNTRACKED_BYTES {
                    continue;
                }
            }
            if let Ok(bytes) = tokio::fs::read(&abs).await {
                let mut line_count = bytes.iter().filter(|b| **b == b'\n').count() as i64;
                if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                    line_count += 1;
                }
                additions += line_count;
            }
        }
    }

    Ok((file_count, additions, deletions))
}
