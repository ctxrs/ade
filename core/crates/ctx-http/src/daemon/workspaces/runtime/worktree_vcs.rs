use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use ctx_core::ids::WorktreeId;
use ctx_core::models::WorktreeVcsSnapshot;
use ctx_workspace_services::worktree_vcs::{
    hydrated_worktree_vcs_snapshot_cache_entry, published_worktree_vcs_snapshot_cache_entry,
};

use crate::daemon::state::{TimedEntry, WorkspaceRuntime};

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

    pub async fn update_worktree_vcs_activity(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        if !self.worktree_vcs_enabled {
            return;
        }
        if previous == next {
            return;
        }
        let mut evicted = Vec::new();
        {
            let mut active = self.worktree_vcs_active.lock().await;
            for worktree_id in previous.difference(next) {
                if let Some(count) = active.get_mut(worktree_id) {
                    if *count <= 1 {
                        active.remove(worktree_id);
                        evicted.push(*worktree_id);
                    } else {
                        *count -= 1;
                    }
                }
            }
            for worktree_id in next.difference(previous) {
                let entry = active.entry(*worktree_id).or_insert(0);
                *entry += 1;
            }
        }
        if !evicted.is_empty() {
            {
                let mut cache = self.worktree_vcs_snapshots.lock().await;
                for worktree_id in &evicted {
                    cache.remove(worktree_id);
                }
            }
            {
                let mut gens = self.worktree_vcs_summary_gen.lock().await;
                for worktree_id in &evicted {
                    gens.remove(worktree_id);
                }
            }
            {
                let mut runtime = self.worktree_vcs_runtime.lock().await;
                for worktree_id in &evicted {
                    runtime.remove(worktree_id);
                }
            }
        }
    }

    pub async fn update_worktree_vcs_open_panes(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        if !self.worktree_vcs_enabled {
            return;
        }
        if previous == next {
            return;
        }
        let mut open = self.worktree_vcs_open_panes.lock().await;
        for worktree_id in previous.difference(next) {
            if let Some(count) = open.get_mut(worktree_id) {
                if *count <= 1 {
                    open.remove(worktree_id);
                } else {
                    *count -= 1;
                }
            }
        }
        for worktree_id in next.difference(previous) {
            let entry = open.entry(*worktree_id).or_insert(0);
            *entry += 1;
        }
    }

    pub async fn worktree_vcs_refresh_lock(
        &self,
        worktree_id: WorktreeId,
    ) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.worktree_vcs_refresh_locks.lock().await;
        match locks.get(&worktree_id).and_then(std::sync::Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(worktree_id, Arc::downgrade(&lock));
                lock
            }
        }
    }

    pub async fn is_worktree_vcs_active(&self, worktree_id: WorktreeId) -> bool {
        if !self.worktree_vcs_enabled {
            return false;
        }
        let active = self.worktree_vcs_active.lock().await;
        active.get(&worktree_id).copied().unwrap_or(0) > 0
    }

    pub async fn is_worktree_vcs_pane_open(&self, worktree_id: WorktreeId) -> bool {
        if !self.worktree_vcs_enabled {
            return false;
        }
        let open = self.worktree_vcs_open_panes.lock().await;
        open.get(&worktree_id).copied().unwrap_or(0) > 0
    }
}
