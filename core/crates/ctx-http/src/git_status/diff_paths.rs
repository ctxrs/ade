use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{Worktree, WorktreeVcsTouchedFile};
use ctx_workspace_services::worktree_vcs::{
    load_diff_file_count_from_source, load_diff_touched_entries_from_source,
    LocalWorktreeVcsSource, SandboxWorktreeVcsSource, WorktreeVcsDiffPathSource,
};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::HttpSandboxWorktreeVcsExecutor;

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
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        let executor = HttpSandboxWorktreeVcsExecutor::new(state, worktree);
        return SandboxWorktreeVcsSource::new(&executor)
            .diff_name_status(base_commit_sha, summary_count)
            .await;
    }
    LocalWorktreeVcsSource::new(worktree, root)
        .diff_name_status(base_commit_sha, summary_count)
        .await
}

async fn load_untracked_paths(state: &Arc<AppState>, worktree: &Worktree) -> Result<Vec<String>> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        let executor = HttpSandboxWorktreeVcsExecutor::new(state, worktree);
        return SandboxWorktreeVcsSource::new(&executor)
            .list_untracked()
            .await;
    }
    LocalWorktreeVcsSource::new(worktree, root)
        .list_untracked()
        .await
}
