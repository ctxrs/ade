use std::collections::HashSet;
use std::sync::Arc;

use crate::daemon::AppState;
use ctx_core::ids::{WorkspaceId, WorktreeId};

pub(super) async fn filter_workspace_worktree_ids(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    worktree_ids: Vec<WorktreeId>,
) -> Vec<WorktreeId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for worktree_id in worktree_ids {
        if !seen.insert(worktree_id) {
            continue;
        }
        let Ok(store) = state.store_for_worktree(worktree_id).await else {
            continue;
        };
        let Ok(Some(worktree)) = store.get_worktree(worktree_id).await else {
            continue;
        };
        if worktree.workspace_id == workspace_id {
            out.push(worktree_id);
        }
    }
    out.sort_by_key(|worktree_id| worktree_id.0);
    out
}
