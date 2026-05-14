use std::sync::Arc;

use ctx_core::models::Worktree;

use crate::daemon::DaemonState;

mod commit_lookup;
mod diff_base;
mod diff_path;
mod status;

pub struct HttpWorktreeVcsSource<'a> {
    state: &'a Arc<DaemonState>,
    worktree: &'a Worktree,
}

impl<'a> HttpWorktreeVcsSource<'a> {
    pub fn new(state: &'a Arc<DaemonState>, worktree: &'a Worktree) -> Self {
        Self { state, worktree }
    }
}
