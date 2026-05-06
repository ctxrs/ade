use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::Worktree;
use ctx_fs::vcs::{self, VcsStructuredStatus};
use ctx_workspace_services::worktree_vcs::{
    is_no_vcs_repo_error, GitStatusEntry, WorktreeVcsGitCommand, WorktreeVcsStatusSource,
    WorktreeVcsStructuredStatus,
};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::{container_git_status_structured, container_git_stdout};
use super::vcs_driver_for_worktree;

pub(super) struct HttpWorktreeVcsStatusSource<'a> {
    state: &'a Arc<AppState>,
    worktree: &'a Worktree,
}

impl<'a> HttpWorktreeVcsStatusSource<'a> {
    pub(super) fn new(state: &'a Arc<AppState>, worktree: &'a Worktree) -> Self {
        Self { state, worktree }
    }
}

#[async_trait::async_trait]
impl WorktreeVcsStatusSource for HttpWorktreeVcsStatusSource<'_> {
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
