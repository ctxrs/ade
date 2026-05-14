use super::super::buffer::VcsPendingBuffer;
use super::super::metrics::VcsStreamMetrics;
use super::runtime::WorkspaceVcsRuntime;
use super::snapshots::seed_current_vcs_snapshots;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    WorktreeVcsStreamClientMessage, WorktreeVcsStreamMessage, WorktreeVcsStreamTier,
};
use ctx_daemon::daemon::WorkspacesHandle;
use std::collections::HashSet;
use std::sync::Arc;

pub(in crate::api::ws::workspace_vcs) async fn handle_workspace_vcs_client_message(
    state: &WorkspacesHandle,
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
            let worktree_ids = state
                .filter_workspace_worktree_ids(workspace_id, worktree_ids)
                .await;
            match tier {
                WorktreeVcsStreamTier::Summary => {
                    state
                        .refresh_worktree_vcs_for_worktrees(&worktree_ids, &[])
                        .await;
                }
                WorktreeVcsStreamTier::Details => {
                    state
                        .refresh_worktree_vcs_for_worktrees(&[], &worktree_ids)
                        .await;
                }
            }
        }
    }
}

async fn replace_workspace_vcs_subscription(
    state: &WorkspacesHandle,
    workspace_id: WorkspaceId,
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    runtime: &mut WorkspaceVcsRuntime,
    summary_worktree_ids: Vec<WorktreeId>,
    detail_worktree_ids: Vec<WorktreeId>,
) {
    let previous_active = runtime.active_worktree_ids();
    let previous_details = runtime.detail_worktree_ids.clone();
    let summary_worktree_ids = state
        .filter_workspace_worktree_ids(workspace_id, summary_worktree_ids)
        .await;
    let detail_worktree_ids = state
        .filter_workspace_worktree_ids(workspace_id, detail_worktree_ids)
        .await;
    runtime.summary_worktree_ids = summary_worktree_ids.iter().copied().collect();
    runtime.detail_worktree_ids = detail_worktree_ids.iter().copied().collect();
    let next_active = runtime.active_worktree_ids();
    let summary_seed_worktree_ids: HashSet<_> = runtime
        .summary_worktree_ids
        .difference(&previous_active)
        .copied()
        .collect();
    let detail_seed_worktree_ids: HashSet<_> = runtime
        .detail_worktree_ids
        .difference(&previous_details)
        .copied()
        .collect();
    let mut summary_refresh_worktree_ids: Vec<_> = runtime
        .summary_worktree_ids
        .difference(&previous_active)
        .copied()
        .collect();
    let mut detail_refresh_worktree_ids: Vec<_> = runtime
        .detail_worktree_ids
        .difference(&previous_details)
        .copied()
        .collect();
    summary_refresh_worktree_ids.sort_by_key(|worktree_id| worktree_id.0);
    detail_refresh_worktree_ids.sort_by_key(|worktree_id| worktree_id.0);
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
        &summary_seed_worktree_ids,
        &detail_seed_worktree_ids,
    )
    .await;
    state
        .refresh_worktree_vcs_for_worktrees(
            &summary_refresh_worktree_ids,
            &detail_refresh_worktree_ids,
        )
        .await;
}
