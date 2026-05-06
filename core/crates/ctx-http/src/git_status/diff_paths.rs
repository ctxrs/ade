use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{Worktree, WorktreeVcsTouchedFile};
use ctx_workspace_services::worktree_vcs::{
    load_diff_file_count_from_source, load_diff_touched_entries_from_source,
    WorktreeVcsDiffPathSource,
};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::{
    container_git_diff_name_status, container_git_diff_name_status_no_renames,
    container_git_list_untracked,
};
use super::vcs_driver_for_worktree;

struct HttpWorktreeVcsDiffPathSource<'a> {
    state: &'a Arc<AppState>,
    worktree: &'a Worktree,
}

#[async_trait::async_trait]
impl WorktreeVcsDiffPathSource for HttpWorktreeVcsDiffPathSource<'_> {
    async fn diff_name_status(
        &self,
        base_commit_sha: &str,
        summary_count: bool,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        load_diff_path_entries_with_mode(self.state, self.worktree, base_commit_sha, summary_count)
            .await
    }

    async fn list_untracked(&self) -> Result<Vec<String>> {
        load_untracked_paths(self.state, self.worktree).await
    }
}

pub(super) async fn load_diff_touched_entries(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<WorktreeVcsTouchedFile>> {
    let source = HttpWorktreeVcsDiffPathSource { state, worktree };
    load_diff_touched_entries_from_source(&source, base_commit_sha).await
}

pub(super) async fn load_diff_file_count(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<i64> {
    let source = HttpWorktreeVcsDiffPathSource { state, worktree };
    load_diff_file_count_from_source(&source, base_commit_sha).await
}

async fn load_diff_path_entries_with_mode(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
    summary_count: bool,
) -> Result<Vec<(String, String, Option<String>)>> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    let entries: Vec<(String, String, Option<String>)> =
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            if summary_count {
                container_git_diff_name_status_no_renames(state, worktree, base_commit_sha).await?
            } else {
                container_git_diff_name_status(state, worktree, base_commit_sha).await?
            }
        } else {
            let driver = vcs_driver_for_worktree(worktree);
            let entries = if summary_count {
                driver
                    .diff_name_status_for_summary(root, base_commit_sha)
                    .await?
            } else {
                driver.diff_name_status(root, base_commit_sha).await?
            };
            entries
                .into_iter()
                .map(|entry| (entry.status, entry.path, entry.orig_path))
                .collect()
        };
    Ok(entries)
}

async fn load_untracked_paths(state: &Arc<AppState>, worktree: &Worktree) -> Result<Vec<String>> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    Ok(
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            container_git_list_untracked(state, worktree).await?
        } else {
            let driver = vcs_driver_for_worktree(worktree);
            driver.list_untracked(root).await?
        },
    )
}
