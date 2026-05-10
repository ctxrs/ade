use std::collections::HashSet;
use std::sync::Arc;

use crate::daemon::AppState;
use ctx_core::ids::WorktreeId;

#[derive(Default)]
pub(in crate::api::ws::workspace_vcs) struct WorkspaceVcsRuntime {
    pub(in crate::api::ws::workspace_vcs) demand_generation: i64,
    pub(in crate::api::ws::workspace_vcs) summary_worktree_ids: HashSet<WorktreeId>,
    pub(in crate::api::ws::workspace_vcs) detail_worktree_ids: HashSet<WorktreeId>,
}

impl WorkspaceVcsRuntime {
    pub(in crate::api::ws::workspace_vcs) fn active_worktree_ids(&self) -> HashSet<WorktreeId> {
        self.summary_worktree_ids
            .union(&self.detail_worktree_ids)
            .copied()
            .collect()
    }
}

pub(in crate::api::ws::workspace_vcs) async fn release_workspace_vcs_demand(
    state: &Arc<AppState>,
    runtime: &WorkspaceVcsRuntime,
) {
    let active = runtime.active_worktree_ids();
    if active.is_empty() && runtime.detail_worktree_ids.is_empty() {
        return;
    }
    state
        .update_worktree_vcs_activity(&active, &HashSet::new())
        .await;
    state
        .update_worktree_vcs_open_panes(&runtime.detail_worktree_ids, &HashSet::new())
        .await;
}
