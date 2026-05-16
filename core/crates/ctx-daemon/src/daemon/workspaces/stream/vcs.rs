use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{Worktree, WorktreeVcsFreshness, WorktreeVcsStreamTier};

use crate::daemon::DaemonState;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceVcsDemandState {
    pub demand_generation: i64,
    pub summary_worktree_ids: HashSet<WorktreeId>,
    pub detail_worktree_ids: HashSet<WorktreeId>,
}

impl WorkspaceVcsDemandState {
    pub fn active_worktree_ids(&self) -> HashSet<WorktreeId> {
        self.summary_worktree_ids
            .union(&self.detail_worktree_ids)
            .copied()
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceVcsSubscriptionPlan {
    pub state: WorkspaceVcsDemandState,
    pub summary_seed_worktree_ids: HashSet<WorktreeId>,
    pub detail_seed_worktree_ids: HashSet<WorktreeId>,
    pub summary_refresh_worktree_ids: Vec<WorktreeId>,
    pub detail_refresh_worktree_ids: Vec<WorktreeId>,
    pub summary_subscribed_worktree_ids: Vec<WorktreeId>,
    pub detail_subscribed_worktree_ids: Vec<WorktreeId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceVcsRefreshPlan {
    pub summary_refresh_worktree_ids: Vec<WorktreeId>,
    pub detail_refresh_worktree_ids: Vec<WorktreeId>,
}

pub async fn filter_workspace_worktree_ids(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    worktree_ids: Vec<WorktreeId>,
) -> Vec<WorktreeId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for worktree_id in worktree_ids {
        if !seen.insert(worktree_id) {
            continue;
        }
        let Some(worktree) = load_worktree(state, worktree_id).await else {
            continue;
        };
        if worktree.workspace_id == workspace_id {
            out.push(worktree_id);
        }
    }
    out.sort_by_key(|worktree_id| worktree_id.0);
    out
}

pub async fn plan_workspace_vcs_subscription_update(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    current: WorkspaceVcsDemandState,
    summary_worktree_ids: Vec<WorktreeId>,
    detail_worktree_ids: Vec<WorktreeId>,
) -> WorkspaceVcsSubscriptionPlan {
    let previous_active = current.active_worktree_ids();
    let previous_details = current.detail_worktree_ids.clone();
    let summary_worktree_ids =
        filter_workspace_worktree_ids(state, workspace_id, summary_worktree_ids).await;
    let detail_worktree_ids =
        filter_workspace_worktree_ids(state, workspace_id, detail_worktree_ids).await;
    let summary_set = summary_worktree_ids.iter().copied().collect::<HashSet<_>>();
    let detail_set = detail_worktree_ids.iter().copied().collect::<HashSet<_>>();
    let next = WorkspaceVcsDemandState {
        demand_generation: current.demand_generation + 1,
        summary_worktree_ids: summary_set,
        detail_worktree_ids: detail_set,
    };
    let next_active = next.active_worktree_ids();

    let summary_seed_worktree_ids = next
        .summary_worktree_ids
        .difference(&previous_active)
        .copied()
        .collect::<HashSet<_>>();
    let detail_seed_worktree_ids = next
        .detail_worktree_ids
        .difference(&previous_details)
        .copied()
        .collect::<HashSet<_>>();
    let summary_refresh_worktree_ids = sorted_worktree_ids(
        next.summary_worktree_ids
            .difference(&previous_active)
            .copied(),
    );
    let detail_refresh_worktree_ids = sorted_worktree_ids(
        next.detail_worktree_ids
            .difference(&previous_details)
            .copied(),
    );

    state
        .update_worktree_vcs_activity(&previous_active, &next_active)
        .await;
    state
        .update_worktree_vcs_open_panes(&previous_details, &next.detail_worktree_ids)
        .await;

    WorkspaceVcsSubscriptionPlan {
        state: next,
        summary_seed_worktree_ids,
        detail_seed_worktree_ids,
        summary_refresh_worktree_ids,
        detail_refresh_worktree_ids,
        summary_subscribed_worktree_ids: summary_worktree_ids,
        detail_subscribed_worktree_ids: detail_worktree_ids,
    }
}

pub async fn plan_workspace_vcs_refresh(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    worktree_ids: Vec<WorktreeId>,
    tier: WorktreeVcsStreamTier,
) -> WorkspaceVcsRefreshPlan {
    let worktree_ids = filter_workspace_worktree_ids(state, workspace_id, worktree_ids).await;
    match tier {
        WorktreeVcsStreamTier::Summary => WorkspaceVcsRefreshPlan {
            summary_refresh_worktree_ids: worktree_ids,
            detail_refresh_worktree_ids: Vec::new(),
        },
        WorktreeVcsStreamTier::Details => WorkspaceVcsRefreshPlan {
            summary_refresh_worktree_ids: Vec::new(),
            detail_refresh_worktree_ids: worktree_ids,
        },
    }
}

pub async fn release_workspace_vcs_demand(
    state: &Arc<DaemonState>,
    demand: &WorkspaceVcsDemandState,
) {
    let active = demand.active_worktree_ids();
    if active.is_empty() && demand.detail_worktree_ids.is_empty() {
        return;
    }
    state
        .update_worktree_vcs_activity(&active, &HashSet::new())
        .await;
    state
        .update_worktree_vcs_open_panes(&demand.detail_worktree_ids, &HashSet::new())
        .await;
}

pub async fn refresh_worktree_vcs_for_worktrees(
    state: &Arc<DaemonState>,
    summary_worktree_ids: &[WorktreeId],
    detail_worktree_ids: &[WorktreeId],
) {
    if !state.worktree_vcs_enabled() {
        return;
    }
    if summary_worktree_ids.is_empty() && detail_worktree_ids.is_empty() {
        return;
    }
    let mut worktrees: HashMap<WorktreeId, (Worktree, bool)> = HashMap::new();
    for worktree_id in summary_worktree_ids {
        if let Some(worktree) = load_worktree(state, *worktree_id).await {
            worktrees.entry(worktree.id).or_insert((worktree, false));
        }
    }
    for worktree_id in detail_worktree_ids {
        if let Some(worktree) = load_worktree(state, *worktree_id).await {
            worktrees
                .entry(worktree.id)
                .and_modify(|(_, details)| *details = true)
                .or_insert((worktree, true));
        }
    }

    for (worktree_id, (worktree, details)) in worktrees {
        state.ensure_git_status_watcher(worktree.clone()).await;
        let should_refresh = !matches!(
            state.get_worktree_vcs_snapshot(worktree.id).await,
            Some(snapshot)
                if snapshot.freshness == WorktreeVcsFreshness::Fresh
                    && snapshot.available
                    && (!details
                        || matches!(
                            snapshot.touched_files_state,
                            ctx_core::models::WorktreeVcsTouchedFilesState::Ready
                        ))
        );
        if should_refresh {
            if let Err(err) =
                crate::daemon::git_status::request_worktree_vcs_refresh_without_transient(
                    state, &worktree, true, details,
                )
                .await
            {
                tracing::warn!(
                    worktree_id = %worktree_id.0,
                    "worktree vcs refresh failed: {err:#}"
                );
            }
        }
    }
}

async fn load_worktree(state: &Arc<DaemonState>, worktree_id: WorktreeId) -> Option<Worktree> {
    let store = state.store_for_worktree(worktree_id).await.ok()?;
    store.get_worktree(worktree_id).await.ok().flatten()
}

fn sorted_worktree_ids<I>(ids: I) -> Vec<WorktreeId>
where
    I: IntoIterator<Item = WorktreeId>,
{
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort_by_key(|worktree_id| worktree_id.0);
    ids
}
