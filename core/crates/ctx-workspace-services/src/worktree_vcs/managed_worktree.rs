use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context};
use ctx_core::ids::{WorkspaceId, WorktreeId};
use tokio::process::Command;

pub fn managed_worktree_path(
    data_root: impl AsRef<Path>,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) -> PathBuf {
    ctx_fs::worktrees::managed_worktree_path(data_root, workspace_id, worktree_id)
}

pub fn matching_managed_worktree_path(
    data_root: impl AsRef<Path>,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: impl AsRef<Path>,
) -> Option<PathBuf> {
    let expected = managed_worktree_path(data_root, workspace_id, worktree_id);
    if normalize_path_for_comparison(worktree_root.as_ref())
        == normalize_path_for_comparison(&expected)
    {
        Some(expected)
    } else {
        None
    }
}

pub async fn create_managed_worktree(
    data_root: impl AsRef<Path>,
    workspace_root: impl AsRef<Path>,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    base_commit_sha: &str,
    branch_name: &str,
) -> anyhow::Result<PathBuf> {
    let canonical_root = managed_worktree_path(data_root, workspace_id, worktree_id);
    if let Some(parent) = canonical_root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    ctx_fs::worktrees::create_worktree(
        workspace_root,
        &canonical_root,
        base_commit_sha,
        branch_name,
    )
    .await?;
    Ok(canonical_root)
}

pub async fn branch_exists(
    workspace_root: impl AsRef<Path>,
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

pub async fn is_git_worktree(worktree_path: impl AsRef<Path>) -> anyhow::Result<bool> {
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

pub async fn remove_worktree(
    workspace_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
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

pub async fn delete_worktree_branch(
    workspace_root: impl AsRef<Path>,
    branch_name: &str,
) -> anyhow::Result<()> {
    ctx_fs::git::delete_branch(workspace_root, branch_name).await
}

fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let mut suffix = Vec::new();
    let mut cursor = path;
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(canonical) => {
                let mut normalized = canonical;
                for component in suffix.iter().rev() {
                    normalized.push(component);
                }
                return normalized;
            }
            Err(_) => {
                let Some(parent) = cursor.parent() else {
                    return path.to_path_buf();
                };
                let Some(name) = cursor.file_name() else {
                    return path.to_path_buf();
                };
                suffix.push(name.to_os_string());
                cursor = parent;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(args: &[&str], cwd: &Path) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn matching_managed_worktree_path_accepts_equivalent_existing_parent() {
        let data_root = tempfile::tempdir().expect("data root");
        let workspace_id = WorkspaceId::new();
        let worktree_id = WorktreeId::new();
        let expected = managed_worktree_path(data_root.path(), workspace_id, worktree_id);
        std::fs::create_dir_all(expected.parent().expect("parent")).expect("create parent");

        let matched = matching_managed_worktree_path(
            data_root.path(),
            workspace_id,
            worktree_id,
            expected.as_path(),
        )
        .expect("managed path");

        assert_eq!(matched, expected);
    }

    #[test]
    fn matching_managed_worktree_path_rejects_external_root() {
        let data_root = tempfile::tempdir().expect("data root");
        let external_root = tempfile::tempdir().expect("external root");

        assert!(matching_managed_worktree_path(
            data_root.path(),
            WorkspaceId::new(),
            WorktreeId::new(),
            external_root.path(),
        )
        .is_none());
    }

    #[tokio::test]
    async fn delete_worktree_branch_removes_existing_branch() {
        let repo = tempfile::tempdir().expect("repo");
        git(&["init"], repo.path());
        git(&["symbolic-ref", "HEAD", "refs/heads/main"], repo.path());
        git(&["config", "user.email", "ctx@example.com"], repo.path());
        git(&["config", "user.name", "Ctx Test"], repo.path());
        std::fs::write(repo.path().join("README.md"), "hello\n").expect("write readme");
        git(&["add", "README.md"], repo.path());
        git(&["commit", "-m", "initial"], repo.path());
        git(&["branch", "stale"], repo.path());
        assert!(branch_exists(repo.path(), "stale")
            .await
            .expect("branch exists"));

        delete_worktree_branch(repo.path(), "stale")
            .await
            .expect("delete branch");

        assert!(!branch_exists(repo.path(), "stale")
            .await
            .expect("branch removed"));
    }
}

pub async fn prune_worktrees(workspace_root: impl AsRef<Path>) -> anyhow::Result<()> {
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
