use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::WorktreeId;

use crate::daemon::state::WorkspaceRuntime;

impl WorkspaceRuntime {
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
