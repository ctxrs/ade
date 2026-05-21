use ctx_core::ids::WorktreeId;
use ctx_core::models::{
    WorktreeVcsSnapshot, WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_worktree_vcs_service::{
    claim_next_worktree_vcs_job, finish_worktree_vcs_job, finish_worktree_vcs_refresh,
    mark_worktree_vcs_runtime_dirty, pending_worktree_vcs_snapshot_cache_entry,
    publish_worktree_vcs_snapshot_cache_entry, published_worktree_vcs_snapshot_cache_entry,
    queue_worktree_vcs_refresh, GitStatusSnapshot, WorktreeVcsDirtyBits, WorktreeVcsSchedulerJob,
    WorktreeVcsSnapshotPublishPolicy,
};
use tokio::sync::broadcast;
use tokio::sync::OwnedSemaphorePermit;

use crate::daemon::state::{TimedEntry, WorkspaceRuntime};

mod activity;

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

    pub async fn queue_worktree_vcs_refresh(
        &self,
        worktree_id: WorktreeId,
        summary: bool,
        touched_files: bool,
    ) {
        if !self.worktree_vcs_enabled {
            return;
        }
        let mut runtime = self.worktree_vcs_runtime.lock().await;
        let entry = runtime.entry(worktree_id).or_default();
        queue_worktree_vcs_refresh(entry, summary, touched_files);
    }

    pub async fn mark_worktree_vcs_dirty(
        &self,
        worktree_id: WorktreeId,
        dirty_bits: WorktreeVcsDirtyBits,
        candidate_paths: Vec<String>,
        pane_open: bool,
    ) {
        if !self.worktree_vcs_enabled {
            return;
        }
        let mut runtime = self.worktree_vcs_runtime.lock().await;
        let entry = runtime.entry(worktree_id).or_default();
        mark_worktree_vcs_runtime_dirty(entry, dirty_bits, candidate_paths, pane_open);
    }

    pub async fn finish_worktree_vcs_refresh(
        &self,
        worktree_id: WorktreeId,
        git_snapshot: GitStatusSnapshot,
        touched_files: WorktreeVcsTouchedFiles,
        touched_files_state: WorktreeVcsTouchedFilesState,
    ) {
        if !self.worktree_vcs_enabled {
            return;
        }
        let mut runtime = self.worktree_vcs_runtime.lock().await;
        let entry = runtime.entry(worktree_id).or_default();
        finish_worktree_vcs_refresh(entry, git_snapshot, touched_files, touched_files_state);
    }

    pub async fn upsert_worktree_vcs_snapshot(
        &self,
        snapshot: WorktreeVcsSnapshot,
        force_emit: bool,
        summary_at: Option<std::time::Instant>,
    ) -> Option<WorktreeVcsSnapshot> {
        if !self.worktree_vcs_enabled {
            return None;
        }
        let now = std::time::Instant::now();
        let policy = WorktreeVcsSnapshotPublishPolicy::default();
        let active = self.worktree_vcs_active.lock().await;
        if active.get(&snapshot.worktree_id).copied().unwrap_or(0) == 0 {
            return None;
        }
        let mut cache = self.worktree_vcs_snapshots.lock().await;
        let entry = cache.entry(snapshot.worktree_id).or_insert_with(|| {
            TimedEntry::new(pending_worktree_vcs_snapshot_cache_entry(
                snapshot.clone(),
                now,
                policy,
            ))
        });
        entry.touch_at(now);
        publish_worktree_vcs_snapshot_cache_entry(
            &mut entry.value,
            snapshot,
            now,
            force_emit,
            summary_at,
            policy,
        )
    }

    pub fn publish_worktree_vcs_event(&self, snapshot: WorktreeVcsSnapshot) {
        let _ = self.worktree_vcs_events.send(snapshot);
    }

    pub fn subscribe_worktree_vcs_events(&self) -> broadcast::Receiver<WorktreeVcsSnapshot> {
        self.worktree_vcs_events.subscribe()
    }

    pub async fn claim_next_worktree_vcs_job(&self) -> Option<WorktreeVcsSchedulerJob> {
        if !self.worktree_vcs_enabled {
            return None;
        }
        let active = self.worktree_vcs_active.lock().await;
        let open = self.worktree_vcs_open_panes.lock().await;
        let mut runtime = self.worktree_vcs_runtime.lock().await;
        claim_next_worktree_vcs_job(&mut runtime, &active, &open)
    }

    pub async fn finish_worktree_vcs_job(&self, worktree_id: WorktreeId) -> bool {
        if !self.worktree_vcs_enabled {
            return false;
        }
        let mut runtime = self.worktree_vcs_runtime.lock().await;
        finish_worktree_vcs_job(&mut runtime, worktree_id)
    }

    pub async fn wait_worktree_vcs_scheduler_notification(&self) {
        self.worktree_vcs_scheduler.notify.notified().await;
    }

    pub fn notify_worktree_vcs_scheduler(&self) {
        self.worktree_vcs_scheduler.notify.notify_one();
    }

    pub fn try_acquire_worktree_vcs_scheduler_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.worktree_vcs_scheduler
            .permits
            .clone()
            .try_acquire_owned()
            .ok()
    }

    pub fn mark_worktree_vcs_scheduler_started(&self) -> bool {
        !self
            .worktree_vcs_scheduler
            .started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    }
}
