use std::time::Duration;

use ctx_core::ids::WorktreeId;
use ctx_core::models::WorktreeVcsSnapshot;
use ctx_workspace_services::worktree_vcs::{
    hydrated_worktree_vcs_snapshot_cache_entry, published_worktree_vcs_snapshot_cache_entry,
};

use crate::daemon::state::{TimedEntry, WorkspaceRuntime};

mod activity;

const HYDRATED_WORKTREE_VCS_CACHE_BACKDATE: Duration = Duration::from_secs(10);

fn hydrated_worktree_vcs_cache_seed_instant(now: std::time::Instant) -> std::time::Instant {
    now.checked_sub(HYDRATED_WORKTREE_VCS_CACHE_BACKDATE)
        .unwrap_or(now)
}

impl WorkspaceRuntime {
    pub async fn cache_worktree_vcs_snapshot(&self, snapshot: WorktreeVcsSnapshot) {
        if !self.worktree_vcs_enabled {
            return;
        }
        let worktree_id = snapshot.worktree_id;
        let now = std::time::Instant::now();
        let mut cache = self.worktree_vcs_snapshots.lock().await;
        cache.insert(
            worktree_id,
            TimedEntry::new(published_worktree_vcs_snapshot_cache_entry(snapshot, now)),
        );
    }

    pub async fn hydrate_worktree_vcs_snapshots(&self, snapshots: Vec<WorktreeVcsSnapshot>) {
        if !self.worktree_vcs_enabled || snapshots.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let seed_instant = hydrated_worktree_vcs_cache_seed_instant(now);
        let mut cache = self.worktree_vcs_snapshots.lock().await;
        for snapshot in snapshots {
            cache.entry(snapshot.worktree_id).or_insert_with(|| {
                TimedEntry::new(hydrated_worktree_vcs_snapshot_cache_entry(
                    snapshot,
                    seed_instant,
                ))
            });
        }
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        if !self.worktree_vcs_enabled {
            return None;
        }
        let mut cache = self.worktree_vcs_snapshots.lock().await;
        cache.get_mut(&worktree_id).map(|entry| {
            entry.touch();
            entry.value.snapshot.clone()
        })
    }
}
