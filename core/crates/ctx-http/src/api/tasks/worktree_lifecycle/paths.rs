use std::path::PathBuf;

use ctx_core::models::{Workspace, Worktree};
use ctx_workspace_services::worktree_vcs::matching_managed_worktree_path;

use crate::daemon::AppState;

pub(crate) fn managed_worktree_root(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Option<PathBuf> {
    matching_managed_worktree_path(
        &state.core.data_root,
        workspace.id,
        worktree.id,
        PathBuf::from(&worktree.root_path),
    )
}
