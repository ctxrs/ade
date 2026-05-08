mod active_cache;
mod bootstrap;
mod cleanup;
mod git_watchers;
mod task_events;
mod worktree_vcs;

use ctx_core::models::{WorktreeVcsComputeState, WorktreeVcsFreshness, WorktreeVcsSnapshot};

pub(crate) fn normalize_hydrated_worktree_vcs_snapshot(
    mut snapshot: WorktreeVcsSnapshot,
) -> WorktreeVcsSnapshot {
    if snapshot.compute_state == WorktreeVcsComputeState::Computing {
        snapshot.compute_state = WorktreeVcsComputeState::Ready;
    }
    snapshot.git_status.raw.clear();
    snapshot.git_status.entries.clear();
    snapshot.freshness = match snapshot.compute_state {
        WorktreeVcsComputeState::Error => WorktreeVcsFreshness::Error,
        WorktreeVcsComputeState::Ready | WorktreeVcsComputeState::Computing => {
            WorktreeVcsFreshness::Stale
        }
    };
    snapshot
}
