use std::collections::HashSet;
use std::sync::Arc;

use crate::daemon::workspaces::stream::refresh_worktree_vcs_for_worktrees;

use super::buffer::{VcsPendingBuffer, VcsSnapshotKey};
use super::metrics::VcsStreamMetrics;
use super::*;

#[derive(Default)]
pub(super) struct WorkspaceVcsRuntime {
    pub(super) demand_generation: i64,
    pub(super) summary_worktree_ids: HashSet<WorktreeId>,
    pub(super) detail_worktree_ids: HashSet<WorktreeId>,
}

impl WorkspaceVcsRuntime {
    fn active_worktree_ids(&self) -> HashSet<WorktreeId> {
        self.summary_worktree_ids
            .union(&self.detail_worktree_ids)
            .copied()
            .collect()
    }
}

pub(super) async fn handle_workspace_vcs_client_message(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    runtime: &mut WorkspaceVcsRuntime,
    message: WorktreeVcsStreamClientMessage,
) {
    match message {
        WorktreeVcsStreamClientMessage::ReplaceSubscription {
            summary_worktree_ids,
            detail_worktree_ids,
        } => {
            replace_workspace_vcs_subscription(
                state,
                workspace_id,
                pending,
                metrics,
                runtime,
                summary_worktree_ids,
                detail_worktree_ids,
            )
            .await;
        }
        WorktreeVcsStreamClientMessage::Refresh { worktree_ids, tier } => {
            let worktree_ids =
                filter_workspace_worktree_ids(state, workspace_id, worktree_ids).await;
            match tier {
                WorktreeVcsStreamTier::Summary => {
                    refresh_worktree_vcs_for_worktrees(state, &worktree_ids, &[]).await;
                }
                WorktreeVcsStreamTier::Details => {
                    refresh_worktree_vcs_for_worktrees(state, &[], &worktree_ids).await;
                }
            }
        }
    }
}

async fn replace_workspace_vcs_subscription(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    runtime: &mut WorkspaceVcsRuntime,
    summary_worktree_ids: Vec<WorktreeId>,
    detail_worktree_ids: Vec<WorktreeId>,
) {
    let previous_active = runtime.active_worktree_ids();
    let previous_details = runtime.detail_worktree_ids.clone();
    let summary_worktree_ids =
        filter_workspace_worktree_ids(state, workspace_id, summary_worktree_ids).await;
    let detail_worktree_ids =
        filter_workspace_worktree_ids(state, workspace_id, detail_worktree_ids).await;
    runtime.summary_worktree_ids = summary_worktree_ids.iter().copied().collect();
    runtime.detail_worktree_ids = detail_worktree_ids.iter().copied().collect();
    let next_active = runtime.active_worktree_ids();
    state
        .update_worktree_vcs_activity(&previous_active, &next_active)
        .await;
    state
        .update_worktree_vcs_open_panes(&previous_details, &runtime.detail_worktree_ids)
        .await;
    runtime.demand_generation += 1;

    pending
        .push_control(WorktreeVcsStreamMessage::Subscribed {
            workspace_id,
            demand_generation: runtime.demand_generation,
            summary_worktree_ids: summary_worktree_ids.clone(),
            detail_worktree_ids: detail_worktree_ids.clone(),
        })
        .await;
    seed_current_vcs_snapshots(
        state,
        workspace_id,
        pending,
        metrics,
        runtime.demand_generation,
        &runtime.summary_worktree_ids,
        &runtime.detail_worktree_ids,
    )
    .await;
    refresh_worktree_vcs_for_worktrees(state, &summary_worktree_ids, &detail_worktree_ids).await;
}

pub(super) async fn release_workspace_vcs_demand(
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

pub(super) async fn seed_current_vcs_snapshots(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    demand_generation: i64,
    summary_worktree_ids: &HashSet<WorktreeId>,
    detail_worktree_ids: &HashSet<WorktreeId>,
) {
    let mut worktree_ids: Vec<_> = summary_worktree_ids
        .union(detail_worktree_ids)
        .copied()
        .collect();
    worktree_ids.sort_by_key(|worktree_id| worktree_id.0);
    for worktree_id in worktree_ids {
        let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree_id).await else {
            continue;
        };
        let tier = if detail_worktree_ids.contains(&worktree_id) {
            WorktreeVcsStreamTier::Details
        } else {
            WorktreeVcsStreamTier::Summary
        };
        queue_vcs_snapshot(
            pending,
            metrics,
            workspace_id,
            demand_generation,
            tier,
            snapshot,
        )
        .await;
    }
}

pub(super) async fn queue_vcs_snapshot(
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    workspace_id: WorkspaceId,
    demand_generation: i64,
    tier: WorktreeVcsStreamTier,
    snapshot: WorktreeVcsSnapshot,
) {
    let worktree_id = snapshot.worktree_id;
    let message = if snapshot.available {
        match tier {
            WorktreeVcsStreamTier::Summary => WorktreeVcsStreamMessage::SummarySnapshot {
                workspace_id,
                worktree_id,
                demand_generation,
                snapshot,
            },
            WorktreeVcsStreamTier::Details => WorktreeVcsStreamMessage::DetailsSnapshot {
                workspace_id,
                worktree_id,
                demand_generation,
                snapshot,
            },
        }
    } else {
        WorktreeVcsStreamMessage::UnavailableSnapshot {
            workspace_id,
            worktree_id,
            demand_generation,
            snapshot,
        }
    };
    let coalesced = pending
        .push_snapshot(VcsSnapshotKey { worktree_id, tier }, message)
        .await;
    metrics.snapshot_queued(coalesced);
}

async fn filter_workspace_worktree_ids(
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
