use std::sync::Arc;
use std::time::Instant;

use ctx_core::models::{Worktree, WorktreeVcsSnapshot};
use ctx_workspace_services::worktree_vcs::snapshot_for_durable_cache;

use crate::daemon::DaemonState;

async fn persist_worktree_vcs_snapshot(
    state: &Arc<DaemonState>,
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
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    snapshot: WorktreeVcsSnapshot,
    force_emit: bool,
    summary_at: Option<Instant>,
) -> Option<WorktreeVcsSnapshot> {
    let published = state
        .upsert_worktree_vcs_snapshot(snapshot, force_emit, summary_at)
        .await?;
    persist_worktree_vcs_snapshot(state, worktree, &published).await;
    if state.is_worktree_vcs_active(worktree.id).await {
        state.publish_worktree_vcs_event(published.clone());
    }
    Some(published)
}

pub(in crate::daemon::git_status) async fn publish_transient_worktree_vcs_snapshot(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    snapshot: WorktreeVcsSnapshot,
) {
    let Some(snapshot) = state
        .upsert_worktree_vcs_snapshot(snapshot, false, None)
        .await
    else {
        return;
    };
    if state.is_worktree_vcs_active(worktree.id).await {
        state.publish_worktree_vcs_event(snapshot);
    }
}
