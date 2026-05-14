use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::Worktree;
use ctx_workspace_services::worktree_vcs::{
    load_git_status_snapshot_from_source, GitStatusSnapshot,
};

use crate::daemon::DaemonState;

use super::super::source::HttpWorktreeVcsSource;

pub async fn load_git_status_snapshot(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    include_untracked_files: bool,
    include_entries: bool,
) -> Result<GitStatusSnapshot> {
    let source = HttpWorktreeVcsSource::new(state, worktree);
    load_git_status_snapshot_from_source(&source, include_untracked_files, include_entries).await
}
