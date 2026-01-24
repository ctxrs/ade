use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use ctx_core::ids::{WorkspaceId, WorktreeId};

use crate::vcs;

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
    let driver = vcs::driver_for_path(workspace_root.as_ref()).await?;
    driver
        .create_worktree(
            workspace_root.as_ref(),
            worktree_path.as_ref(),
            base_commit_sha,
            branch_name,
        )
        .await
}

pub async fn remove_worktree(
    workspace_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
) -> Result<()> {
    let driver = vcs::driver_for_path(workspace_root.as_ref()).await?;
    driver
        .remove_worktree(workspace_root.as_ref(), worktree_path.as_ref())
        .await?;
    if tokio::fs::metadata(worktree_path.as_ref()).await.is_ok() {
        tokio::fs::remove_dir_all(worktree_path.as_ref())
            .await
            .context("removing worktree dir")?;
    }
    Ok(())
}

pub async fn prune_worktrees(workspace_root: impl AsRef<Path>) -> Result<()> {
    let driver = vcs::driver_for_path(workspace_root.as_ref()).await?;
    driver.prune_worktrees(workspace_root.as_ref()).await
}

pub async fn ensure_worktree_attached(
    workspace_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<()> {
    let driver = vcs::driver_for_path(workspace_root.as_ref()).await?;
    let worktree_path = worktree_path.as_ref();
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating worktree parent dir")?;
    }

    if tokio::fs::metadata(worktree_path).await.is_ok() {
        if driver.is_worktree(worktree_path).await.unwrap_or(false) {
            return Ok(());
        }
        tokio::fs::remove_dir_all(worktree_path)
            .await
            .context("removing stale worktree dir")?;
    }

    driver
        .create_worktree(
            workspace_root.as_ref(),
            worktree_path,
            base_commit_sha,
            branch_name,
        )
        .await
}

pub async fn diff_worktree(
    worktree_path: impl AsRef<Path>,
    base_commit_sha: &str,
) -> Result<String> {
    let root = worktree_path.as_ref();
    let driver = vcs::driver_for_path(root).await?;
    let mut out = driver.diff(root, base_commit_sha).await?;

    // Base diffs do not include untracked files, but we want the UI to show newly created
    // files even before they are staged.
    let untracked = driver.list_untracked(root).await.unwrap_or_default();
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

        if let Ok(patch) = driver.diff_untracked_file(root, &rel).await {
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
    let driver = vcs::driver_for_path(root).await?;
    let (mut file_count, mut additions, deletions) =
        driver.diff_summary(root, base_commit_sha).await?;

    let untracked = driver.list_untracked(root).await.unwrap_or_default();
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
