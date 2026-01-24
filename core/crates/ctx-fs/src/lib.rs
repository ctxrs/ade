pub mod git;
pub mod patch;
pub mod vcs;
pub mod worktrees;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tokio::process::Command;

    use crate::git::{assert_git_repo, rev_parse_head};
    use crate::worktrees::{create_worktree, diff_worktree};

    async fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn worktree_diff_shows_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        run_git(root, &["init"]).await;
        run_git(root, &["config", "user.email", "test@example.com"]).await;
        run_git(root, &["config", "user.name", "Test"]).await;

        fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "."]).await;
        run_git(root, &["commit", "-m", "init"]).await;

        assert_git_repo(root).await.unwrap();
        let base = rev_parse_head(root).await.unwrap();

        let wt_path = root.join("wt1");
        create_worktree(root, &wt_path, &base, "ctx/test")
            .await
            .unwrap();

        fs::write(wt_path.join("file.txt"), "hello\nworld\n").unwrap();

        let diff = diff_worktree(&wt_path, &base).await.unwrap();
        assert!(diff.contains("+world"));
    }
}
