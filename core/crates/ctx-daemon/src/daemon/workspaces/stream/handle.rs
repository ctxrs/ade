use std::collections::{HashMap, HashSet};

use tokio::sync::broadcast;

use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    SessionHeadDelta, SessionHeadSnapshot, SessionSummaryDelta, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotStreamMessage,
};
use ctx_observability::telemetry::TelemetryEvent;
use ctx_workspace_active_snapshot::{SessionReplayCursor, WorkspaceActiveSubscriptionState};

use super::super::{load_workspace_active_snapshot_state, WorkspaceHydrationError};
use super::access::require_existing_workspace_for_stream;
use super::{
    WorkspaceStreamAccessError, WorkspaceStreamRouteAdmission, WorkspaceStreamRouteError,
    WorkspaceStreamRouteParams,
};
use crate::daemon::{WorkspaceStreamHandle, WorkspacesHandle};

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
            .map_err(WorkspaceStreamRouteError::from_stream_access)?;
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
        super::filter_workspace_worktree_ids(&self.state, workspace_id, worktree_ids).await
    }

    pub async fn refresh_worktree_vcs_for_worktrees(
        &self,
        summary_worktree_ids: &[WorktreeId],
        detail_worktree_ids: &[WorktreeId],
    ) {
        super::refresh_worktree_vcs_for_worktrees(
            &self.state,
            summary_worktree_ids,
            detail_worktree_ids,
        )
        .await;
    }

    pub async fn update_worktree_vcs_activity(
        &self,
        previous: &std::collections::HashSet<WorktreeId>,
        next: &std::collections::HashSet<WorktreeId>,
    ) {
        self.state
            .update_worktree_vcs_activity(previous, next)
            .await;
    }

    pub async fn update_worktree_vcs_open_panes(
        &self,
        previous: &std::collections::HashSet<WorktreeId>,
        next: &std::collections::HashSet<WorktreeId>,
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
        current: super::WorkspaceVcsDemandState,
        summary_worktree_ids: Vec<WorktreeId>,
        detail_worktree_ids: Vec<WorktreeId>,
    ) -> super::WorkspaceVcsSubscriptionPlan {
        super::plan_workspace_vcs_subscription_update(
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
        tier: ctx_core::models::WorktreeVcsStreamTier,
    ) -> super::WorkspaceVcsRefreshPlan {
        super::plan_workspace_vcs_refresh(&self.state, workspace_id, worktree_ids, tier).await
    }

    pub async fn release_workspace_vcs_demand(&self, demand: &super::WorkspaceVcsDemandState) {
        super::release_workspace_vcs_demand(&self.state, demand).await;
    }

    pub fn route_workspace_vcs_snapshot(
        &self,
        demand: &super::WorkspaceVcsDemandState,
        worktree_id: WorktreeId,
    ) -> super::WorkspaceVcsSnapshotRoute {
        super::route_workspace_vcs_snapshot(demand, worktree_id)
    }

    pub fn plan_workspace_vcs_lag_reseed(
        &self,
        demand: &super::WorkspaceVcsDemandState,
    ) -> super::WorkspaceVcsLagReseedPlan {
        super::plan_workspace_vcs_lag_reseed(demand)
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

impl WorkspaceStreamHandle {
    pub async fn workspace_exists(&self, workspace_id: WorkspaceId) -> anyhow::Result<bool> {
        self.state
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map(|workspace| workspace.is_some())
    }

    pub async fn require_workspace_active_stream_access(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceStreamAccessError> {
        require_existing_workspace_for_stream(&self.state, workspace_id).await
    }

    pub async fn admit_workspace_active_stream_for_route(
        &self,
        params: WorkspaceStreamRouteParams,
    ) -> Result<WorkspaceStreamRouteAdmission, WorkspaceStreamRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.require_workspace_active_stream_access(workspace_id)
            .await
            .map_err(WorkspaceStreamRouteError::from_stream_access)?;
        Ok(WorkspaceStreamRouteAdmission::new(workspace_id))
    }

    pub async fn subscribe_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Receiver<WorkspaceActiveSnapshotEvent> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .subscribe(workspace_id)
            .await
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
    }

    pub async fn activate_workspace_merge_queue(&self, workspace_id: WorkspaceId) {
        crate::daemon::merge_queue::activate_workspace_merge_queue(&self.state, workspace_id).await;
    }

    pub async fn load_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> (i64, i64) {
        load_workspace_active_snapshot_state(&self.state, workspace_id).await
    }

    pub async fn initial_stream_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> super::WorkspaceStreamInitialState {
        super::initial_stream_state(&self.state, workspace_id).await
    }

    pub async fn load_initial_snapshot_read_model(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<super::WorkspaceStreamSnapshotReadModel, WorkspaceHydrationError> {
        super::load_initial_snapshot_read_model(&self.state, workspace_id).await
    }

    pub async fn workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> WorkspaceActiveSnapshot {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(workspace_id, i64::MAX)
            .await
    }

    pub async fn workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> WorkspaceActiveHeadBatch {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await
    }

    pub async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        self.state
            .workspaces
            .workspace_active_snapshot
            .session_replay_cursor(workspace_id, session_id)
            .await
    }

    pub fn active_head_cursors_from_snapshot_read_model(
        &self,
        read_model: &super::WorkspaceStreamSnapshotReadModel,
    ) -> HashMap<SessionId, SessionReplayCursor> {
        super::active_head_cursors_from_snapshot_read_model(read_model)
    }

    pub async fn plan_workspace_stream_replay_program(
        &self,
        workspace_id: WorkspaceId,
        resolved_sessions: &[super::WorkspaceStreamResolvedSession],
        live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
        include_initial_snapshot: bool,
    ) -> super::WorkspaceStreamReplayProgram {
        super::plan_workspace_stream_replay_program(
            &self.state,
            workspace_id,
            resolved_sessions,
            live_subscriptions,
            active_head_cursors,
            include_initial_snapshot,
        )
        .await
    }

    pub async fn plan_workspace_stream_replay_program_with_step_hook<H>(
        &self,
        workspace_id: WorkspaceId,
        resolved_sessions: &[super::WorkspaceStreamResolvedSession],
        live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
        include_initial_snapshot: bool,
        step_hook: &mut H,
    ) -> Result<super::WorkspaceStreamReplayProgram, H::Error>
    where
        H: super::WorkspaceStreamReplayStepHook,
    {
        super::plan_workspace_stream_replay_program_with_step_hook(
            &self.state,
            workspace_id,
            resolved_sessions,
            live_subscriptions,
            active_head_cursors,
            include_initial_snapshot,
            step_hook,
        )
        .await
    }

    pub fn accept_session_delta_cursor(
        &self,
        current: SessionReplayCursor,
        delta: &SessionHeadDelta,
    ) -> super::WorkspaceStreamCursorAcceptance {
        super::accept_session_delta_cursor(current, delta)
    }

    pub fn accept_session_head_cursor(
        &self,
        current: SessionReplayCursor,
        head: &SessionHeadSnapshot,
    ) -> super::WorkspaceStreamCursorAcceptance {
        super::accept_session_head_cursor(current, head)
    }

    pub fn is_session_head_delta_after_cursor(
        &self,
        delta: &SessionHeadDelta,
        cursor: SessionReplayCursor,
    ) -> bool {
        super::is_session_head_delta_after_cursor(delta, cursor)
    }

    pub fn is_session_summary_delta_after_cursor(
        &self,
        delta: &SessionSummaryDelta,
        cursor: SessionReplayCursor,
    ) -> bool {
        super::is_session_summary_delta_after_cursor(delta, cursor)
    }

    pub fn merge_replayed_and_live_subscription_cursors(
        &self,
        live_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        replayed_subscriptions: HashMap<SessionId, SessionReplayCursor>,
    ) -> HashMap<SessionId, SessionReplayCursor> {
        super::merge_replayed_and_live_subscription_cursors(
            live_subscriptions,
            replayed_subscriptions,
        )
    }

    pub fn finalize_workspace_stream_subscription_replay(
        &self,
        current_state: &WorkspaceActiveSubscriptionState,
        current_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        replayed_subscriptions: HashMap<SessionId, SessionReplayCursor>,
        transaction_sessions: &[super::WorkspaceStreamResolvedSession],
    ) -> super::WorkspaceStreamSubscriptionReplayFinalization {
        super::finalize_workspace_stream_subscription_replay(
            current_state,
            current_subscriptions,
            replayed_subscriptions,
            transaction_sessions,
        )
    }

    pub async fn active_task_subscription_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        super::active_task_subscription_cursor(&self.state, workspace_id, session_id).await
    }

    pub async fn apply_workspace_stream_subscription_event(
        &self,
        workspace_id: WorkspaceId,
        subscription_state: WorkspaceActiveSubscriptionState,
        subscriptions: HashMap<SessionId, SessionReplayCursor>,
        event: &WorkspaceActiveSnapshotEvent,
    ) -> super::WorkspaceStreamSubscriptionEventApplication {
        super::apply_workspace_stream_subscription_event(
            &self.state,
            workspace_id,
            subscription_state,
            subscriptions,
            event,
        )
        .await
    }

    pub async fn apply_workspace_stream_live_event(
        &self,
        workspace_id: WorkspaceId,
        subscription_state: WorkspaceActiveSubscriptionState,
        subscriptions: HashMap<SessionId, SessionReplayCursor>,
        event: WorkspaceActiveSnapshotEvent,
    ) -> super::WorkspaceStreamLiveEventApplication {
        super::apply_workspace_stream_live_event(
            &self.state,
            workspace_id,
            subscription_state,
            subscriptions,
            event,
        )
        .await
    }

    pub fn primary_session_id_for_active_task_event(
        &self,
        task: &ctx_core::models::WorkspaceActiveTaskSummary,
    ) -> SessionId {
        super::primary_session_id_for_active_task_event(task)
    }

    pub fn event_snapshot_rev(&self, event: &WorkspaceActiveSnapshotEvent) -> Option<i64> {
        super::event_snapshot_rev(event)
    }

    pub fn event_blocks_pending_replay(
        &self,
        event: &WorkspaceActiveSnapshotEvent,
        pending_replay_sessions: &HashSet<SessionId>,
        subscription_state: &WorkspaceActiveSubscriptionState,
    ) -> bool {
        super::event_blocks_pending_replay(event, pending_replay_sessions, subscription_state)
    }

    pub fn plan_workspace_stream_event_route(
        &self,
        subscription_state: &WorkspaceActiveSubscriptionState,
        event: WorkspaceActiveSnapshotEvent,
    ) -> super::WorkspaceStreamEventRoutePlan {
        super::plan_workspace_stream_event_route(subscription_state, event)
    }

    pub async fn resolve_workspace_active_snapshot_subscriptions(
        &self,
        workspace_id: WorkspaceId,
        message: WorkspaceActiveSnapshotClientMessage,
        existing: &HashMap<SessionId, SessionReplayCursor>,
    ) -> Result<
        super::WorkspaceStreamSubscriptionPlan,
        super::WorkspaceStreamSubscriptionResolutionError,
    > {
        super::prepare_subscription_read_model(&self.state, workspace_id)
            .await
            .map_err(super::WorkspaceStreamSubscriptionResolutionError::Hydration)?;
        let resolved = super::resolve_workspace_active_snapshot_subscriptions(
            &self.state,
            workspace_id,
            message.clone(),
            existing,
        )
        .await
        .map_err(|_| super::WorkspaceStreamSubscriptionResolutionError::Resolution)?;
        Ok(super::plan_workspace_stream_subscription(
            &message, resolved, existing,
        ))
    }

    pub async fn plan_workspace_stream_subscription_transaction(
        &self,
        workspace_id: WorkspaceId,
        message: WorkspaceActiveSnapshotClientMessage,
        current_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
        current_fingerprint: Option<&str>,
    ) -> Result<
        super::WorkspaceStreamSubscriptionTransactionPlan,
        super::WorkspaceStreamSubscriptionResolutionError,
    > {
        super::prepare_subscription_read_model(&self.state, workspace_id)
            .await
            .map_err(super::WorkspaceStreamSubscriptionResolutionError::Hydration)?;
        let resolved = super::resolve_workspace_active_snapshot_subscriptions(
            &self.state,
            workspace_id,
            message.clone(),
            current_subscriptions,
        )
        .await
        .map_err(|_| super::WorkspaceStreamSubscriptionResolutionError::Resolution)?;
        Ok(super::plan_workspace_stream_subscription_transaction(
            &message,
            resolved,
            current_subscriptions,
            current_fingerprint,
        ))
    }

    pub async fn replay_session_events<F, Fut>(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        after_cursor: SessionReplayCursor,
        list_failpoint: &'static str,
        send_failpoint: Option<&'static str>,
        emit: F,
    ) -> Result<super::ReplayOutcome, ()>
    where
        F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
        Fut: std::future::Future<Output = Result<(), ()>>,
    {
        super::replay_session_events(
            &self.state,
            workspace_id,
            session_id,
            after_cursor,
            list_failpoint,
            send_failpoint,
            emit,
        )
        .await
    }

    pub async fn attach_session_pin(&self, session_id: SessionId) {
        self.state.attach_session(session_id).await;
    }

    pub async fn detach_session_pin(&self, session_id: SessionId) {
        self.state.detach_session(session_id).await;
    }

    pub async fn apply_workspace_stream_session_pin_changes(
        &self,
        pin_changes: &super::WorkspaceStreamSessionPinChanges,
    ) {
        for session_id in &pin_changes.attach {
            self.state.attach_session(*session_id).await;
        }
        for session_id in &pin_changes.detach {
            self.state.detach_session(*session_id).await;
        }
    }

    pub async fn release_workspace_stream_session_pins<I>(&self, session_ids: I)
    where
        I: IntoIterator<Item = SessionId>,
    {
        for session_id in session_ids {
            self.state.detach_session(session_id).await;
        }
    }

    pub async fn emit_workspace_stream_incident(
        &self,
        event_name: &'static str,
        labels: &[(&'static str, serde_json::Value)],
    ) {
        let mut event = TelemetryEvent::daemon_incident(event_name)
            .with_source("workspace_stream")
            .with_property("has_workspace_scope", serde_json::json!(true));
        for (key, value) in labels {
            event = event.with_property(*key, value.clone());
        }
        self.state.telemetry.telemetry.emit(event).await;
    }

    pub async fn record_workspace_stream_receiver_drain(
        &self,
        queue_label: &'static str,
        event_count: usize,
        hit_limit: bool,
    ) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("queue_label".to_string(), queue_label.to_string());
        labels.insert(
            "hit_limit".to_string(),
            if hit_limit { "true" } else { "false" }.to_string(),
        );
        self.state
            .telemetry
            .perf_telemetry
            .record_metric(
                ctx_observability::perf_telemetry::PerfMetric {
                    name: "workspace.stream.receiver_drain_event_count".to_string(),
                    kind: ctx_observability::perf_telemetry::PerfMetricKind::Histogram,
                    unit: "count".to_string(),
                    value: event_count as f64,
                    labels,
                },
                None,
                None,
                None,
            )
            .await;
    }
}
