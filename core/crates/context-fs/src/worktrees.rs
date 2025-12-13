use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use context_core::ids::{WorkspaceId, WorktreeId};

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
            const MAX_BYTES: u64 = 512 * 1024;
            if meta.len() > MAX_BYTES {
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
