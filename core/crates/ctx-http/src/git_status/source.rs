use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::Worktree;
use ctx_fs::vcs::{self, VcsStructuredStatus};
use ctx_workspace_config as workspace_config;
use ctx_workspace_services::worktree_vcs::{
    is_no_vcs_repo_error, GitStatusEntry, WorktreeVcsCommitLookupSource, WorktreeVcsDiffBaseSource,
    WorktreeVcsGitCommand, WorktreeVcsStatusSource, WorktreeVcsStructuredStatus,
};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::{container_git_status_structured, container_git_stdout};
use super::vcs_driver_for_worktree;

pub(crate) struct HttpWorktreeVcsSource<'a> {
    state: &'a Arc<AppState>,
    worktree: &'a Worktree,
}

impl<'a> HttpWorktreeVcsSource<'a> {
    pub(crate) fn new(state: &'a Arc<AppState>, worktree: &'a Worktree) -> Self {
        Self { state, worktree }
    }
}

#[async_trait::async_trait]
impl WorktreeVcsStatusSource for HttpWorktreeVcsSource<'_> {
    async fn has_vcs_repo(&self) -> Result<bool> {
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            return match container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::IsInsideWorkTree,
            )
            .await
            {
                Ok(_) => Ok(true),
                Err(err) if is_no_vcs_repo_error(&err) => Ok(false),
                Err(err) => Err(err),
            };
        }

        let root = data_plane.live_worktree_root.as_path();
        let driver = match vcs::driver_for_path(root).await {
            Ok(driver) => driver,
            Err(err) if is_no_vcs_repo_error(&err) => return Ok(false),
            Err(err) => return Err(err),
        };
        match driver.assert_repo(root).await {
            Ok(()) => Ok(true),
            Err(err) if is_no_vcs_repo_error(&err) => Ok(false),
            Err(err) => Err(err),
        }
    }

    async fn load_structured_status(
        &self,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> Result<WorktreeVcsStructuredStatus> {
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        let structured = if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            container_git_status_structured(
                self.state,
                self.worktree,
                include_untracked_files,
                include_entries,
            )
            .await?
        } else {
            let vcs = vcs_driver_for_worktree(self.worktree);
            vcs.status_structured(root, include_untracked_files, include_entries)
                .await?
        };
        Ok(worktree_vcs_structured_status_from_vcs(structured))
    }
}

#[async_trait::async_trait]
impl WorktreeVcsCommitLookupSource for HttpWorktreeVcsSource<'_> {
    async fn resolve_commit(&self, reference: &str) -> Result<String> {
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let bytes = container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::RevParse {
                    reference: reference.to_string(),
                },
            )
            .await?;
            return Ok(ctx_workspace_services::worktree_vcs::parse_git_single_ref(
                &bytes,
            ));
        }
        let driver = vcs::driver_for_path(root).await?;
        if reference == "HEAD" {
            driver.rev_parse_head(root).await
        } else {
            driver.rev_parse_ref(root, reference).await
        }
    }
}

#[async_trait::async_trait]
impl WorktreeVcsDiffBaseSource for HttpWorktreeVcsSource<'_> {
    async fn load_primary_branch(&self) -> Result<Option<String>> {
        let store = self.state.store_for_worktree(self.worktree.id).await?;
        workspace_config::load_primary_branch(&store).await
    }

    async fn rev_parse_head(&self) -> Result<String> {
        self.resolve_commit("HEAD").await
    }

    async fn rev_parse_refs(&self, references: &[&str]) -> Result<Vec<String>> {
        if references.is_empty() {
            return Ok(Vec::new());
        }
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let bytes = container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::RevParseRefs {
                    references: references
                        .iter()
                        .map(|reference| (*reference).to_string())
                        .collect(),
                },
            )
            .await?;
            return ctx_workspace_services::worktree_vcs::parse_git_refs(&bytes, references.len());
        }

        let driver = vcs::driver_for_path(root).await?;
        let mut commits = Vec::with_capacity(references.len());
        for reference in references {
            let commit = if *reference == "HEAD" {
                driver.rev_parse_head(root).await?
            } else {
                driver.rev_parse_ref(root, reference).await?
            };
            commits.push(commit);
        }
        Ok(commits)
    }

    async fn merge_base(&self, target_branch: &str) -> Result<String> {
        let data_plane = resolve_worktree_data_plane(self.state, self.worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let bytes = container_git_stdout(
                self.state,
                self.worktree,
                WorktreeVcsGitCommand::MergeBase {
                    target_branch: target_branch.to_string(),
                },
            )
            .await?;
            return Ok(ctx_workspace_services::worktree_vcs::parse_git_single_ref(
                &bytes,
            ));
        }
        let driver = vcs::driver_for_path(root).await?;
        driver.merge_base(root, target_branch, "HEAD").await
    }

    fn redact_error(&self, err: &anyhow::Error) -> String {
        crate::logs::redact_sensitive(&err.to_string())
    }
}

fn worktree_vcs_structured_status_from_vcs(
    structured: VcsStructuredStatus,
) -> WorktreeVcsStructuredStatus {
    WorktreeVcsStructuredStatus {
        raw: structured.raw,
        summary_line: structured.branch.summary_line,
        branch: structured.branch.branch,
        upstream: structured.branch.upstream,
        ahead: structured.branch.ahead,
        behind: structured.branch.behind,
        detached: structured.branch.detached,
        staged: structured.staged,
        unstaged: structured.unstaged,
        untracked: structured.untracked,
        entries: structured
            .entries
            .into_iter()
            .map(|entry| GitStatusEntry {
                path: entry.path,
                orig_path: entry.orig_path,
                index_status: entry.index_status,
                worktree_status: entry.worktree_status,
            })
            .collect(),
        entries_total_count: structured.total_count,
        entries_truncated: structured.truncated,
    }
}
