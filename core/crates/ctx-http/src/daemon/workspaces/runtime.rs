use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::watch;

use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Task, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, Worktree, WorktreeVcsComputeState,
    WorktreeVcsFreshness, WorktreeVcsSnapshot,
};

use crate::daemon::state::{
    AppState, TimedEntry, WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
    WorkspaceRuntime, WorktreeBootstrapGate,
};
use crate::git_status;

const HYDRATED_WORKTREE_VCS_CACHE_BACKDATE: Duration = Duration::from_secs(10);

pub(crate) fn normalize_hydrated_worktree_vcs_snapshot(
    mut snapshot: WorktreeVcsSnapshot,
) -> WorktreeVcsSnapshot {
    if snapshot.compute_state == WorktreeVcsComputeState::Computing {
        snapshot.compute_state = WorktreeVcsComputeState::Ready;
    }
    snapshot.git_status.raw.clear();
    snapshot.freshness = match snapshot.compute_state {
        WorktreeVcsComputeState::Error => WorktreeVcsFreshness::Error,
        WorktreeVcsComputeState::Ready | WorktreeVcsComputeState::Computing => {
            WorktreeVcsFreshness::Stale
        }
    };
    snapshot
}

fn hydrated_worktree_vcs_cache_seed_instant(now: std::time::Instant) -> std::time::Instant {
    now.checked_sub(HYDRATED_WORKTREE_VCS_CACHE_BACKDATE)
        .unwrap_or(now)
}

impl WorkspaceRuntime {
    pub async fn cached_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<(i64, i64)> {
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            (
                entry.value.snapshot.snapshot_rev,
                entry.value.snapshot.archived_rev,
            )
        })
    }

    pub async fn cached_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveSnapshot> {
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            entry.value.snapshot.clone()
        })
    }

    pub async fn cache_workspace_active_snapshot(&self, snapshot: WorkspaceActiveSnapshot) {
        let workspace_id = snapshot.workspace_id;
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.insert(
            workspace_id,
            TimedEntry::new(WorkspaceActiveSnapshotCacheEntry { snapshot }),
        );
    }

    pub async fn cached_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveHeadBatch> {
        let mut cache = self.workspace_active_heads_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            entry.value.batch.clone()
        })
    }

    pub async fn cache_workspace_active_heads(&self, batch: WorkspaceActiveHeadBatch) {
        let workspace_id = batch.workspace_id;
        let mut cache = self.workspace_active_heads_cache.lock().await;
        cache.insert(
            workspace_id,
            TimedEntry::new(WorkspaceActiveHeadCacheEntry { batch }),
        );
    }

    pub async fn cache_worktree_vcs_snapshot(&self, snapshot: WorktreeVcsSnapshot) {
        if !self.worktree_vcs_enabled {
            return;
        }
        let worktree_id = snapshot.worktree_id;
        let now = std::time::Instant::now();
        let fingerprint = serde_json::to_string(&snapshot).unwrap_or_default();
        let mut cache = self.worktree_vcs_snapshots.lock().await;
        cache.insert(
            worktree_id,
            TimedEntry::new(crate::daemon::WorktreeVcsSnapshotCacheEntry {
                snapshot,
                fingerprint,
                emitted_at: now,
                last_change_at: now,
                last_summary_at: Some(now),
            }),
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
                crate::daemon::TimedEntry::new(crate::daemon::WorktreeVcsSnapshotCacheEntry {
                    fingerprint: serde_json::to_string(&snapshot).unwrap_or_default(),
                    snapshot,
                    emitted_at: seed_instant,
                    last_change_at: seed_instant,
                    last_summary_at: None,
                })
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
            self.workspace_active_snapshot
                .drop_worktree_vcs_snapshots(&evicted)
                .await;
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
        Arc::clone(
            locks
                .entry(worktree_id)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
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

    pub async fn register_worktree_bootstrap(
        &self,
        worktree_id: WorktreeId,
        wait_for_completion: bool,
    ) {
        let (done_tx, _) = watch::channel(false);
        let mut map = self.worktree_bootstrap_gates.lock().await;
        map.insert(
            worktree_id,
            TimedEntry::new(WorktreeBootstrapGate {
                wait_for_completion,
                done_tx,
            }),
        );
    }

    pub async fn finish_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        let gate = {
            let mut map = self.worktree_bootstrap_gates.lock().await;
            map.remove(&worktree_id)
        };
        if let Some(gate) = gate {
            let _ = gate.value.done_tx.send(true);
        }
    }

    pub async fn wait_for_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        let mut done_rx = {
            let mut map = self.worktree_bootstrap_gates.lock().await;
            let Some(gate) = map.get_mut(&worktree_id) else {
                return;
            };
            gate.touch();
            if !gate.value.wait_for_completion {
                return;
            }
            gate.value.done_tx.subscribe()
        };
        if *done_rx.borrow() {
            return;
        }
        let _ = done_rx.changed().await;
    }

    pub async fn cleanup_workspace(&self, state: &AppState, workspace_id: WorkspaceId) {
        let session_ids = {
            let cache = state.sessions.session_meta_cache.lock().await;
            cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if entry.value.workspace_id == workspace_id {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        for session_id in session_ids {
            state.cleanup_session(session_id).await;
        }
        {
            let mut cache = self.workspace_active_snapshot_cache.lock().await;
            cache.remove(&workspace_id);
        }
        {
            let mut cache = self.workspace_active_heads_cache.lock().await;
            cache.remove(&workspace_id);
        }
        {
            let mut cache = self.workspace_file_completions_cache.lock().await;
            cache.remove(&workspace_id);
        }
        self.workspace_active_snapshot
            .remove_workspace(workspace_id)
            .await;
        state.core.stores.evict_workspace(workspace_id).await;
    }

    pub async fn emit_workspace_task_upsert(
        &self,
        state: &AppState,
        task_id: TaskId,
    ) -> Result<()> {
        let mut task: Option<Task> = None;
        let store = state.store_for_task(task_id).await?;
        match store.get_workspace_active_task_summary(task_id).await? {
            Some(summary) => {
                let workspace_id = summary.task.workspace_id;
                task = Some(summary.task.clone());
                self.workspace_active_snapshot
                    .publish_active_task_upsert(workspace_id, summary)
                    .await;
            }
            None => {
                if let Some(loaded) = store.get_task(task_id).await? {
                    task = Some(loaded.clone());
                    self.workspace_active_snapshot
                        .publish_active_task_delete(loaded.workspace_id, task_id)
                        .await;
                }
            }
        }

        if let Some(task) = task.as_ref().filter(|task| task.archived_at.is_some()) {
            let _ = self.emit_workspace_archived_task_upsert(state, task).await;
        }
        Ok(())
    }

    pub async fn emit_workspace_task_delete(
        &self,
        state: &AppState,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        if let Err(err) = state.store_for_workspace(workspace_id).await {
            tracing::warn!(
                workspace_id = %workspace_id.0,
                task_id = %task_id.0,
                "workspace task delete store missing: {err:#}"
            );
            return;
        }
        self.workspace_active_snapshot
            .publish_active_task_delete(workspace_id, task_id)
            .await;
    }

    pub async fn emit_workspace_archived_task_delete(
        &self,
        state: &AppState,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        let updated = match state.store_for_workspace(workspace_id).await {
            Ok(store) => match store
                .bump_workspace_archived_snapshot_rev(workspace_id)
                .await
            {
                Ok(_) => true,
                Err(err) => {
                    tracing::warn!(
                        workspace_id = %workspace_id.0,
                        task_id = %task_id.0,
                        "workspace archived delete read model update failed: {err:#}"
                    );
                    false
                }
            },
            Err(err) => {
                tracing::warn!(
                    workspace_id = %workspace_id.0,
                    task_id = %task_id.0,
                    "workspace archived delete read model store missing: {err:#}"
                );
                false
            }
        };
        if updated {
            self.workspace_active_snapshot
                .publish_archived_task_delete(workspace_id, task_id)
                .await;
        }
    }

    async fn emit_workspace_archived_task_upsert(
        &self,
        state: &AppState,
        task: &Task,
    ) -> Result<()> {
        let store = state.store_for_task(task.id).await?;
        let Some(summary) = store.get_workspace_task_summary(task.id).await? else {
            return Ok(());
        };
        if summary.task.archived_at.is_none() {
            return Ok(());
        }

        let _ = store
            .bump_workspace_archived_snapshot_rev(task.workspace_id)
            .await?;
        self.workspace_active_snapshot
            .publish_archived_task_upsert(task.workspace_id, summary)
            .await;
        Ok(())
    }

    pub async fn ensure_git_status_watcher(&self, state: &Arc<AppState>, worktree: Worktree) {
        if !self.worktree_vcs_enabled {
            return;
        }
        let mut watchers = self.git_status_watchers.lock().await;
        if !watchers.insert(worktree.id) {
            return;
        }
        let worktree_id = worktree.id;
        let state = Arc::clone(state);
        tokio::spawn(async move {
            if let Err(err) = git_status::run_git_status_watcher(state.clone(), worktree).await {
                tracing::warn!(worktree_id = %worktree_id.0, "git status watcher failed: {err:#}");
            }
            state
                .workspaces
                .release_git_status_watcher(worktree_id)
                .await;
        });
    }

    pub async fn release_git_status_watcher(&self, worktree_id: WorktreeId) {
        let mut watchers = self.git_status_watchers.lock().await;
        watchers.remove(&worktree_id);
    }
}
