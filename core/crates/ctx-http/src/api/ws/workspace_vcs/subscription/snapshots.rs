use std::collections::HashSet;
use std::sync::Arc;

use super::super::buffer::{VcsPendingBuffer, VcsSnapshotKey};
use super::super::metrics::VcsStreamMetrics;
use crate::daemon::WorkspacesHandle;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{WorktreeVcsSnapshot, WorktreeVcsStreamMessage, WorktreeVcsStreamTier};

pub(in crate::api::ws::workspace_vcs) async fn seed_current_vcs_snapshots(
    state: &WorkspacesHandle,
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

pub(in crate::api::ws::workspace_vcs) async fn queue_vcs_snapshot(
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
