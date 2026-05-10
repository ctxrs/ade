use std::sync::Arc;

use super::super::buffer::VcsPendingBuffer;
use super::super::metrics::VcsStreamMetrics;
use super::filter::filter_workspace_worktree_ids;
use super::runtime::WorkspaceVcsRuntime;
use super::snapshots::seed_current_vcs_snapshots;
use crate::daemon::workspaces::stream::refresh_worktree_vcs_for_worktrees;
use crate::daemon::AppState;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    WorktreeVcsStreamClientMessage, WorktreeVcsStreamMessage, WorktreeVcsStreamTier,
};

pub(in crate::api::ws::workspace_vcs) async fn handle_workspace_vcs_client_message(
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
