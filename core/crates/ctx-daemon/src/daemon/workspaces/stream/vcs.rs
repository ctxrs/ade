use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{Worktree, WorktreeVcsFreshness, WorktreeVcsStreamTier};
use ctx_route_contracts::workspaces::{WorkspaceStreamRouteError, WorkspaceStreamRouteParams};
use ctx_workspace_stream_service::vcs as stream_vcs_service;
pub use ctx_workspace_stream_service::vcs::{
    plan_workspace_vcs_lag_reseed, route_workspace_vcs_snapshot, WorkspaceVcsDemandState,
    WorkspaceVcsLagReseedPlan, WorkspaceVcsRefreshPlan, WorkspaceVcsSnapshotRoute,
    WorkspaceVcsSnapshotSeed, WorkspaceVcsSubscriptionPlan,
};
use tokio::sync::broadcast;

use crate::daemon::DaemonState;
use crate::daemon::WorkspacesHandle;

use super::access::{
    require_existing_workspace_for_stream, workspace_stream_route_error_from_access,
};
use super::{WorkspaceStreamAccessError, WorkspaceStreamRouteAdmission};

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
    let plan = stream_vcs_service::plan_workspace_vcs_subscription_update(
        current,
        summary_worktree_ids,
        detail_worktree_ids,
    );
    let next_active = plan.state.active_worktree_ids();

    state
        .update_worktree_vcs_activity(&previous_active, &next_active)
        .await;
    state
        .update_worktree_vcs_open_panes(&previous_details, &plan.state.detail_worktree_ids)
        .await;

    plan
}

pub async fn plan_workspace_vcs_refresh(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    worktree_ids: Vec<WorktreeId>,
    tier: WorktreeVcsStreamTier,
) -> WorkspaceVcsRefreshPlan {
    let worktree_ids = filter_workspace_worktree_ids(state, workspace_id, worktree_ids).await;
    stream_vcs_service::plan_workspace_vcs_refresh(worktree_ids, tier)
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

impl WorkspacesHandle {
    pub async fn require_workspace_vcs_stream_access(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceStreamAccessError> {
        require_existing_workspace_for_stream(&self.state, workspace_id).await
    }

    pub async fn admit_workspace_vcs_stream_for_route(
        &self,
        params: WorkspaceStreamRouteParams,
    ) -> Result<WorkspaceStreamRouteAdmission, WorkspaceStreamRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.require_workspace_vcs_stream_access(workspace_id)
            .await
            .map_err(workspace_stream_route_error_from_access)?;
        Ok(WorkspaceStreamRouteAdmission::new(workspace_id))
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<ctx_core::models::WorktreeVcsSnapshot> {
        self.state.get_worktree_vcs_snapshot(worktree_id).await
    }

    pub fn subscribe_worktree_vcs_events(
        &self,
    ) -> broadcast::Receiver<ctx_core::models::WorktreeVcsSnapshot> {
        self.state.subscribe_worktree_vcs_events()
    }

    pub async fn filter_workspace_worktree_ids(
        &self,
        workspace_id: WorkspaceId,
        worktree_ids: Vec<WorktreeId>,
    ) -> Vec<WorktreeId> {
        filter_workspace_worktree_ids(&self.state, workspace_id, worktree_ids).await
    }

    pub async fn refresh_worktree_vcs_for_worktrees(
        &self,
        summary_worktree_ids: &[WorktreeId],
        detail_worktree_ids: &[WorktreeId],
    ) {
        refresh_worktree_vcs_for_worktrees(&self.state, summary_worktree_ids, detail_worktree_ids)
            .await;
    }

    pub async fn update_worktree_vcs_activity(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        self.state
            .update_worktree_vcs_activity(previous, next)
            .await;
    }

    pub async fn update_worktree_vcs_open_panes(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        self.state
            .update_worktree_vcs_open_panes(previous, next)
            .await;
    }

    #[cfg(test)]
    pub async fn is_worktree_vcs_active_for_test(&self, worktree_id: WorktreeId) -> bool {
        self.state.is_worktree_vcs_active(worktree_id).await
    }

    #[cfg(test)]
    pub async fn is_worktree_vcs_pane_open_for_test(&self, worktree_id: WorktreeId) -> bool {
        self.state.is_worktree_vcs_pane_open(worktree_id).await
    }

    pub async fn plan_workspace_vcs_subscription_update(
        &self,
        workspace_id: WorkspaceId,
        current: WorkspaceVcsDemandState,
        summary_worktree_ids: Vec<WorktreeId>,
        detail_worktree_ids: Vec<WorktreeId>,
    ) -> WorkspaceVcsSubscriptionPlan {
        plan_workspace_vcs_subscription_update(
            &self.state,
            workspace_id,
            current,
            summary_worktree_ids,
            detail_worktree_ids,
        )
        .await
    }

    pub async fn plan_workspace_vcs_refresh(
        &self,
        workspace_id: WorkspaceId,
        worktree_ids: Vec<WorktreeId>,
        tier: WorktreeVcsStreamTier,
    ) -> WorkspaceVcsRefreshPlan {
        plan_workspace_vcs_refresh(&self.state, workspace_id, worktree_ids, tier).await
    }

    pub async fn release_workspace_vcs_demand(&self, demand: &WorkspaceVcsDemandState) {
        release_workspace_vcs_demand(&self.state, demand).await;
    }

    pub fn route_workspace_vcs_snapshot(
        &self,
        demand: &WorkspaceVcsDemandState,
        worktree_id: WorktreeId,
    ) -> WorkspaceVcsSnapshotRoute {
        route_workspace_vcs_snapshot(demand, worktree_id)
    }

    pub fn plan_workspace_vcs_lag_reseed(
        &self,
        demand: &WorkspaceVcsDemandState,
    ) -> WorkspaceVcsLagReseedPlan {
        plan_workspace_vcs_lag_reseed(demand)
    }

    pub async fn record_workspace_vcs_stream_metric(&self, name: &str, value: u64) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("stream".to_string(), "workspace_vcs".to_string());
        let metric = ctx_observability::perf_telemetry::PerfMetric {
            name: name.to_string(),
            kind: ctx_observability::perf_telemetry::PerfMetricKind::Counter,
            unit: "count".to_string(),
            value: value as f64,
            labels,
        };
        self.state
            .telemetry
            .perf_telemetry
            .record_metric(metric, None, None, None)
            .await;
    }
}
