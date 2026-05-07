use std::path::Path;

use anyhow::Result;
use ctx_core::models::Worktree;
use ctx_fs::vcs;

use super::WorktreeVcsStructuredStatus;
use super::{is_no_vcs_repo_error, worktree_vcs_structured_status_from_vcs};

pub struct LocalWorktreeVcsSource<'a> {
    worktree: &'a Worktree,
    root: &'a Path,
}

impl<'a> LocalWorktreeVcsSource<'a> {
    pub fn new(worktree: &'a Worktree, root: &'a Path) -> Self {
        Self { worktree, root }
    }

    pub async fn has_vcs_repo(&self) -> Result<bool> {
        let driver = match vcs::driver_for_path(self.root).await {
            Ok(driver) => driver,
            Err(err) if is_no_vcs_repo_error(&err) => return Ok(false),
            Err(err) => return Err(err),
        };
        match driver.assert_repo(self.root).await {
            Ok(()) => Ok(true),
            Err(err) if is_no_vcs_repo_error(&err) => Ok(false),
            Err(err) => Err(err),
        }
    }

    pub async fn load_structured_status(
        &self,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> Result<WorktreeVcsStructuredStatus> {
        let driver = vcs::driver_for_kind(self.worktree.vcs_kind.clone());
        Ok(worktree_vcs_structured_status_from_vcs(
            driver
                .status_structured(self.root, include_untracked_files, include_entries)
                .await?,
        ))
    }

    pub async fn resolve_commit(&self, reference: &str) -> Result<String> {
        let driver = vcs::driver_for_path(self.root).await?;
        if reference == "HEAD" {
            driver.rev_parse_head(self.root).await
        } else {
            driver.rev_parse_ref(self.root, reference).await
        }
    }

    pub async fn rev_parse_refs(&self, references: &[&str]) -> Result<Vec<String>> {
        if references.is_empty() {
            return Ok(Vec::new());
        }
        let driver = vcs::driver_for_path(self.root).await?;
        let mut commits = Vec::with_capacity(references.len());
        for reference in references {
            let commit = if *reference == "HEAD" {
                driver.rev_parse_head(self.root).await?
            } else {
                driver.rev_parse_ref(self.root, reference).await?
            };
            commits.push(commit);
        }
        Ok(commits)
    }

    pub async fn merge_base(&self, target_branch: &str) -> Result<String> {
        let driver = vcs::driver_for_path(self.root).await?;
        driver.merge_base(self.root, target_branch, "HEAD").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use ctx_core::models::VcsKind;

    fn worktree(root: &Path) -> Worktree {
        Worktree {
            id: WorktreeId::new(),
            workspace_id: WorkspaceId::new(),
            root_path: root.to_string_lossy().to_string(),
            base_commit_sha: "base".to_string(),
            git_branch: Some("main".to_string()),
            vcs_kind: Some(VcsKind::Git),
            base_revision: None,
            vcs_ref: None,
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        }
    }

    fn git(args: &[&str], cwd: &Path) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    #[tokio::test]
    async fn local_source_reports_missing_repo_false() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = worktree(temp.path());
        let source = LocalWorktreeVcsSource::new(&worktree, temp.path());

        assert!(!source.has_vcs_repo().await.expect("check repo"));
    }

    #[tokio::test]
    async fn local_source_resolves_git_status_and_refs() {
        let temp = tempfile::tempdir().expect("tempdir");
        git(&["init"], temp.path());
        git(&["symbolic-ref", "HEAD", "refs/heads/main"], temp.path());
        git(&["config", "user.email", "ctx@example.com"], temp.path());
        git(&["config", "user.name", "Ctx Test"], temp.path());
        std::fs::write(temp.path().join("README.md"), "hello\n").expect("write readme");
        git(&["add", "README.md"], temp.path());
        git(&["commit", "-m", "initial"], temp.path());
        std::fs::write(temp.path().join("README.md"), "changed\n").expect("modify readme");

        let worktree = worktree(temp.path());
        let source = LocalWorktreeVcsSource::new(&worktree, temp.path());

        assert!(source.has_vcs_repo().await.expect("check repo"));
        let head = source.resolve_commit("HEAD").await.expect("resolve head");
        assert_eq!(
            source
                .rev_parse_refs(&["HEAD"])
                .await
                .expect("resolve refs"),
            vec![head]
        );
        let status = source
            .load_structured_status(true, true)
            .await
            .expect("load status");
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.unstaged, 1);
        assert_eq!(status.entries.len(), 1);
    }
}
