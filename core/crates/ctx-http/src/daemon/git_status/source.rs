use std::sync::Arc;

use ctx_core::models::Worktree;

use crate::daemon::AppState;

mod commit_lookup;
mod diff_base;
mod diff_path;
mod status;

pub(crate) struct HttpWorktreeVcsSource<'a> {
    state: &'a Arc<AppState>,
    worktree: &'a Worktree,
}

impl<'a> HttpWorktreeVcsSource<'a> {
    pub(crate) fn new(state: &'a Arc<AppState>, worktree: &'a Worktree) -> Self {
        Self { state, worktree }
    }
}
