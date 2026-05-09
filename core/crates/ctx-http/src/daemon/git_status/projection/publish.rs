use std::sync::Arc;
use std::time::Instant;

use ctx_core::models::{Worktree, WorktreeVcsSnapshot};
use ctx_workspace_services::worktree_vcs::{
    pending_worktree_vcs_snapshot_cache_entry, publish_worktree_vcs_snapshot_cache_entry,
    snapshot_for_durable_cache, WorktreeVcsSnapshotPublishPolicy,
};

use crate::daemon::AppState;

async fn persist_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    snapshot: &WorktreeVcsSnapshot,
) {
    let durable = snapshot_for_durable_cache(snapshot);
    let Ok(store) = state.store_for_worktree(worktree.id).await else {
        return;
    };
    if let Err(err) = store
        .upsert_worktree_vcs_snapshot_cache(worktree, &durable)
        .await
    {
        tracing::warn!(
            worktree_id = %worktree.id.0,
            "persisting worktree vcs snapshot cache failed: {err:#}"
        );
    }
}

pub(in crate::daemon::git_status) async fn publish_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    snapshot: WorktreeVcsSnapshot,
    force_emit: bool,
    summary_at: Option<Instant>,
) -> Option<WorktreeVcsSnapshot> {
    let published = upsert_worktree_vcs_snapshot(state, snapshot, force_emit, summary_at).await?;
    persist_worktree_vcs_snapshot(state, worktree, &published).await;
    if state.is_worktree_vcs_active(worktree.id).await {
        let _ = state.workspaces.worktree_vcs_events.send(published.clone());
    }
    Some(published)
}

async fn upsert_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    snapshot: WorktreeVcsSnapshot,
    force_emit: bool,
    summary_at: Option<Instant>,
) -> Option<WorktreeVcsSnapshot> {
    let now = Instant::now();
    let policy = WorktreeVcsSnapshotPublishPolicy::default();
    let active = state.workspaces.worktree_vcs_active.lock().await;
    if active.get(&snapshot.worktree_id).copied().unwrap_or(0) == 0 {
        return None;
    }
    let mut cache = state.workspaces.worktree_vcs_snapshots.lock().await;
    let entry = cache.entry(snapshot.worktree_id).or_insert_with(|| {
        crate::daemon::TimedEntry::new(pending_worktree_vcs_snapshot_cache_entry(
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

pub(in crate::daemon::git_status) async fn publish_transient_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    snapshot: WorktreeVcsSnapshot,
) {
    let Some(snapshot) = upsert_worktree_vcs_snapshot(state, snapshot, false, None).await else {
        return;
    };
    if state.is_worktree_vcs_active(worktree.id).await {
        let _ = state.workspaces.worktree_vcs_events.send(snapshot);
    }
}
