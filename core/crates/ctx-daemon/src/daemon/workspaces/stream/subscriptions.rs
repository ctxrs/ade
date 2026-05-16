use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    TaskDeltaKind, WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotSessionIntent,
};
use ctx_workspace_active_snapshot::{
    resolve_workspace_active_snapshot_subscriptions as resolve_workspace_active_snapshot_subscriptions_with_source,
    ResolvedWorkspaceActiveSessionReplay, ResolvedWorkspaceActiveSessionSubscription,
    ResolvedWorkspaceActiveSubscriptions, SessionReplayCursor, WorkspaceActiveSubscriptionSource,
    WorkspaceActiveSubscriptionState,
};

use crate::daemon::workspaces::WorkspaceHydrationError;
use crate::daemon::DaemonState;

use super::cursor_acceptance::{accept_session_delta_cursor, accept_session_head_cursor};
use super::event_routing::{
    plan_workspace_stream_event_route, primary_session_id_for_active_task_event,
    WorkspaceStreamEventRoutePlan,
};
use super::replay_cursor::active_task_subscription_cursor;

#[derive(Debug)]
pub enum WorkspaceStreamSubscriptionResolutionError {
    Hydration(WorkspaceHydrationError),
    Resolution,
}

#[derive(Clone, Debug)]
pub struct WorkspaceStreamSubscriptionPlan {
    pub include_initial_snapshot: bool,
    pub fingerprint: String,
    pub sessions: Vec<WorkspaceStreamResolvedSession>,
    pub state: WorkspaceActiveSubscriptionState,
    pub provisional_subscriptions: HashMap<SessionId, SessionReplayCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStreamSessionPinChanges {
    pub attach: Vec<SessionId>,
    pub detach: Vec<SessionId>,
}

#[derive(Clone, Debug)]
pub enum WorkspaceStreamSubscriptionTransactionPlan {
    NoChange,
    Apply(WorkspaceStreamSubscriptionApplyPlan),
}

#[derive(Clone, Debug)]
pub struct WorkspaceStreamSubscriptionApplyPlan {
    pub include_initial_snapshot: bool,
    pub fingerprint: String,
    pub sessions: Vec<WorkspaceStreamResolvedSession>,
    pub state: WorkspaceActiveSubscriptionState,
    pub provisional_subscriptions: HashMap<SessionId, SessionReplayCursor>,
    pub pin_changes: WorkspaceStreamSessionPinChanges,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStreamSubscriptionReplayFinalization {
    pub subscriptions: HashMap<SessionId, SessionReplayCursor>,
    pub pin_changes: WorkspaceStreamSessionPinChanges,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStreamSubscriptionEventApplication {
    pub state: WorkspaceActiveSubscriptionState,
    pub subscriptions: HashMap<SessionId, SessionReplayCursor>,
    pub pin_changes: WorkspaceStreamSessionPinChanges,
    pub should_route: bool,
}

#[derive(Debug)]
pub struct WorkspaceStreamLiveEventApplication {
    pub state: WorkspaceActiveSubscriptionState,
    pub subscriptions: HashMap<SessionId, SessionReplayCursor>,
    pub pin_changes: WorkspaceStreamSessionPinChanges,
    pub route_plan: WorkspaceStreamEventRoutePlan,
}

#[derive(Clone, Debug)]
pub struct WorkspaceStreamResolvedSession {
    pub session_id: SessionId,
    pub intent: WorkspaceActiveSnapshotSessionIntent,
    pub replay: WorkspaceStreamSessionReplay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceStreamSessionReplay {
    Reset,
    Resume {
        after_seq: i64,
        after_projection_rev: i64,
    },
}

pub fn plan_workspace_stream_subscription(
    message: &WorkspaceActiveSnapshotClientMessage,
    resolved: ResolvedWorkspaceActiveSubscriptions,
    existing: &HashMap<SessionId, SessionReplayCursor>,
) -> WorkspaceStreamSubscriptionPlan {
    let include_initial_snapshot = matches!(
        message,
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            include_active_heads: true,
            ..
        }
    );
    let ResolvedWorkspaceActiveSubscriptions { sessions, state } = resolved;
    let sessions = sessions
        .into_iter()
        .map(WorkspaceStreamResolvedSession::from)
        .collect::<Vec<_>>();
    let fingerprint =
        workspace_subscription_fingerprint(include_initial_snapshot, &sessions, &state);
    let provisional_subscriptions =
        provisional_subscriptions_for_resolved_sessions(&sessions, existing);
    WorkspaceStreamSubscriptionPlan {
        include_initial_snapshot,
        fingerprint,
        sessions,
        state,
        provisional_subscriptions,
    }
}

pub fn plan_workspace_stream_subscription_transaction(
    message: &WorkspaceActiveSnapshotClientMessage,
    resolved: ResolvedWorkspaceActiveSubscriptions,
    current_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
    current_fingerprint: Option<&str>,
) -> WorkspaceStreamSubscriptionTransactionPlan {
    let plan = plan_workspace_stream_subscription(message, resolved, current_subscriptions);
    if current_fingerprint == Some(plan.fingerprint.as_str()) {
        return WorkspaceStreamSubscriptionTransactionPlan::NoChange;
    }
    let pin_changes = workspace_stream_session_pin_changes(
        current_subscriptions.keys().copied(),
        plan.provisional_subscriptions.keys().copied(),
    );
    WorkspaceStreamSubscriptionTransactionPlan::Apply(WorkspaceStreamSubscriptionApplyPlan {
        include_initial_snapshot: plan.include_initial_snapshot,
        fingerprint: plan.fingerprint,
        sessions: plan.sessions,
        state: plan.state,
        provisional_subscriptions: plan.provisional_subscriptions,
        pin_changes,
    })
}

pub fn finalize_workspace_stream_subscription_replay(
    current_state: &WorkspaceActiveSubscriptionState,
    current_subscriptions: &HashMap<SessionId, SessionReplayCursor>,
    replayed_subscriptions: HashMap<SessionId, SessionReplayCursor>,
    transaction_sessions: &[WorkspaceStreamResolvedSession],
) -> WorkspaceStreamSubscriptionReplayFinalization {
    let head_targets = transaction_sessions
        .iter()
        .filter_map(|subscription| {
            (subscription.intent == WorkspaceActiveSnapshotSessionIntent::Head)
                .then_some(subscription.session_id)
        })
        .collect::<HashSet<_>>();
    let mut subscriptions = current_subscriptions
        .iter()
        .map(|(session_id, live_cursor)| {
            let last_sent = replayed_subscriptions
                .get(session_id)
                .map(|replayed_cursor| replayed_cursor.cover(*live_cursor))
                .unwrap_or(*live_cursor);
            (*session_id, last_sent)
        })
        .collect::<HashMap<_, _>>();
    for (session_id, replayed_cursor) in replayed_subscriptions {
        if subscriptions.contains_key(&session_id) {
            continue;
        }
        if head_targets.contains(&session_id)
            && subscription_state_contains_session(current_state, session_id)
        {
            subscriptions.insert(session_id, replayed_cursor);
        }
    }
    let pin_changes = workspace_stream_session_pin_changes(
        current_subscriptions.keys().copied(),
        subscriptions.keys().copied(),
    );
    WorkspaceStreamSubscriptionReplayFinalization {
        subscriptions,
        pin_changes,
    }
}

pub async fn apply_workspace_stream_subscription_event(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    mut subscription_state: WorkspaceActiveSubscriptionState,
    mut subscriptions: HashMap<SessionId, SessionReplayCursor>,
    event: &WorkspaceActiveSnapshotEvent,
) -> WorkspaceStreamSubscriptionEventApplication {
    let previous_subscriptions = subscriptions.keys().copied().collect::<HashSet<_>>();
    if let WorkspaceActiveSnapshotEvent::SessionRemoved { session_id, .. } = event {
        let removed_explicit = subscription_state.explicit_sessions.remove(session_id);
        let removed_foreground = subscription_state
            .foreground_session_ids
            .as_mut()
            .map(|foreground| foreground.remove(session_id))
            .unwrap_or(false);
        if subscription_state
            .foreground_session_ids
            .as_ref()
            .is_some_and(HashSet::is_empty)
        {
            subscription_state.foreground_session_ids = None;
        }
        subscription_state.replay_sessions.remove(session_id);
        let removed_subscription = subscriptions.remove(session_id).is_some();
        return subscription_event_application(
            subscription_state,
            subscriptions,
            previous_subscriptions,
            removed_explicit || removed_foreground || removed_subscription,
        );
    }

    if !subscription_state.active_scope {
        return subscription_event_application(
            subscription_state,
            subscriptions,
            previous_subscriptions,
            true,
        );
    }

    match event {
        WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
            let session_id = primary_session_id_for_active_task_event(task);
            subscription_state
                .active_task_sessions
                .insert(task.task.id, session_id);
            if let std::collections::hash_map::Entry::Vacant(entry) =
                subscriptions.entry(session_id)
            {
                let last_sent =
                    active_task_subscription_cursor(state, workspace_id, session_id).await;
                entry.insert(last_sent);
            }
        }
        WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => {
            remove_active_task_subscription_if_unused(
                &mut subscription_state,
                &mut subscriptions,
                *task_id,
            );
        }
        WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
            if matches!(delta.kind, TaskDeltaKind::Archived) =>
        {
            remove_active_task_subscription_if_unused(
                &mut subscription_state,
                &mut subscriptions,
                delta.task.id,
            );
        }
        _ => {}
    }
    subscription_event_application(
        subscription_state,
        subscriptions,
        previous_subscriptions,
        true,
    )
}

pub async fn apply_workspace_stream_live_event(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    subscription_state: WorkspaceActiveSubscriptionState,
    subscriptions: HashMap<SessionId, SessionReplayCursor>,
    event: WorkspaceActiveSnapshotEvent,
) -> WorkspaceStreamLiveEventApplication {
    let application = apply_workspace_stream_subscription_event(
        state,
        workspace_id,
        subscription_state,
        subscriptions,
        &event,
    )
    .await;
    let WorkspaceStreamSubscriptionEventApplication {
        state,
        mut subscriptions,
        pin_changes,
        should_route,
    } = application;
    if !should_route {
        return WorkspaceStreamLiveEventApplication {
            state,
            subscriptions,
            pin_changes,
            route_plan: WorkspaceStreamEventRoutePlan::Drop,
        };
    }

    let accepted = match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
            let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                return WorkspaceStreamLiveEventApplication {
                    state,
                    subscriptions,
                    pin_changes,
                    route_plan: WorkspaceStreamEventRoutePlan::Drop,
                };
            };
            let accepted = accept_session_delta_cursor(*cursor, delta);
            if !accepted.accepted {
                false
            } else {
                *cursor = accepted.next_cursor;
                true
            }
        }
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
            let Some(cursor) = subscriptions.get_mut(&head.session.id) else {
                return WorkspaceStreamLiveEventApplication {
                    state,
                    subscriptions,
                    pin_changes,
                    route_plan: WorkspaceStreamEventRoutePlan::Drop,
                };
            };
            let accepted = accept_session_head_cursor(*cursor, head);
            if !accepted.accepted {
                false
            } else {
                *cursor = accepted.next_cursor;
                true
            }
        }
        _ => true,
    };
    let route_plan = if accepted {
        plan_workspace_stream_event_route(&state, event)
    } else {
        WorkspaceStreamEventRoutePlan::Drop
    };

    WorkspaceStreamLiveEventApplication {
        state,
        subscriptions,
        pin_changes,
        route_plan,
    }
}

fn workspace_stream_session_pin_changes<I, J>(
    current: I,
    next: J,
) -> WorkspaceStreamSessionPinChanges
where
    I: IntoIterator<Item = SessionId>,
    J: IntoIterator<Item = SessionId>,
{
    let current = current.into_iter().collect::<HashSet<_>>();
    let next = next.into_iter().collect::<HashSet<_>>();
    let mut attach = next.difference(&current).copied().collect::<Vec<_>>();
    let mut detach = current.difference(&next).copied().collect::<Vec<_>>();
    attach.sort_by_key(|session_id| session_id.0);
    detach.sort_by_key(|session_id| session_id.0);
    WorkspaceStreamSessionPinChanges { attach, detach }
}

fn subscription_state_contains_session(
    state: &WorkspaceActiveSubscriptionState,
    session_id: SessionId,
) -> bool {
    state.explicit_sessions.contains(&session_id)
        || state.replay_sessions.contains(&session_id)
        || state
            .active_task_sessions
            .values()
            .any(|active_session_id| *active_session_id == session_id)
        || state
            .foreground_session_ids
            .as_ref()
            .is_some_and(|foreground| foreground.contains(&session_id))
}

fn remove_active_task_subscription_if_unused(
    subscription_state: &mut WorkspaceActiveSubscriptionState,
    subscriptions: &mut HashMap<SessionId, SessionReplayCursor>,
    task_id: ctx_core::ids::TaskId,
) {
    if let Some(session_id) = subscription_state.active_task_sessions.remove(&task_id) {
        let still_active = subscription_state
            .active_task_sessions
            .values()
            .any(|id| *id == session_id);
        if !still_active && !subscription_state.explicit_sessions.contains(&session_id) {
            subscriptions.remove(&session_id);
        }
    }
}

fn subscription_event_application(
    state: WorkspaceActiveSubscriptionState,
    subscriptions: HashMap<SessionId, SessionReplayCursor>,
    previous_subscriptions: HashSet<SessionId>,
    should_route: bool,
) -> WorkspaceStreamSubscriptionEventApplication {
    let next_subscriptions = subscriptions.keys().copied().collect::<HashSet<_>>();
    let pin_changes = workspace_stream_session_pin_changes(
        previous_subscriptions.into_iter(),
        next_subscriptions.into_iter(),
    );
    WorkspaceStreamSubscriptionEventApplication {
        state,
        subscriptions,
        pin_changes,
        should_route,
    }
}

fn workspace_subscription_fingerprint(
    include_initial_snapshot: bool,
    sessions: &[WorkspaceStreamResolvedSession],
    next_state: &WorkspaceActiveSubscriptionState,
) -> String {
    let mut sessions = sessions
        .iter()
        .map(|subscription| {
            let replay = match subscription.replay {
                WorkspaceStreamSessionReplay::Reset => "reset".to_string(),
                WorkspaceStreamSessionReplay::Resume {
                    after_seq,
                    after_projection_rev,
                } => format!("resume:{after_seq}:{after_projection_rev}"),
            };
            format!(
                "{}:{:?}:{}",
                subscription.session_id.0, subscription.intent, replay
            )
        })
        .collect::<Vec<_>>();
    sessions.sort();
    let mut foreground = next_state
        .foreground_session_ids
        .as_ref()
        .map(|ids| ids.iter().map(|id| id.0.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();
    foreground.sort();
    format!(
        "heads={};active={};foreground={};sessions={}",
        include_initial_snapshot,
        next_state.active_scope,
        foreground.join(","),
        sessions.join("|")
    )
}

fn provisional_subscriptions_for_resolved_sessions(
    sessions: &[WorkspaceStreamResolvedSession],
    existing: &HashMap<SessionId, SessionReplayCursor>,
) -> HashMap<SessionId, SessionReplayCursor> {
    let mut provisional_subscriptions = HashMap::new();
    for subscription in sessions {
        let WorkspaceStreamSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        } = subscription.replay
        else {
            continue;
        };
        let requested = SessionReplayCursor {
            last_event_seq: after_seq.max(0),
            projection_rev: after_projection_rev.max(0),
        };
        let last_sent = existing
            .get(&subscription.session_id)
            .copied()
            .map(|existing| existing.cover(requested))
            .unwrap_or(requested);
        provisional_subscriptions.insert(subscription.session_id, last_sent);
    }
    provisional_subscriptions
}

impl From<ResolvedWorkspaceActiveSessionSubscription> for WorkspaceStreamResolvedSession {
    fn from(value: ResolvedWorkspaceActiveSessionSubscription) -> Self {
        Self {
            session_id: value.session_id,
            intent: value.intent,
            replay: value.replay.into(),
        }
    }
}

impl From<ResolvedWorkspaceActiveSessionReplay> for WorkspaceStreamSessionReplay {
    fn from(value: ResolvedWorkspaceActiveSessionReplay) -> Self {
        match value {
            ResolvedWorkspaceActiveSessionReplay::Reset => Self::Reset,
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq,
                after_projection_rev,
            } => Self::Resume {
                after_seq,
                after_projection_rev,
            },
        }
    }
}

pub async fn resolve_workspace_active_snapshot_subscriptions(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    existing: &HashMap<SessionId, SessionReplayCursor>,
) -> Result<ResolvedWorkspaceActiveSubscriptions, ()> {
    resolve_workspace_active_snapshot_subscriptions_with_source(
        &HttpWorkspaceActiveSubscriptionSource { state },
        workspace_id,
        message,
        existing,
    )
    .await
}

struct HttpWorkspaceActiveSubscriptionSource<'a> {
    state: &'a Arc<DaemonState>,
}

impl WorkspaceActiveSubscriptionSource for HttpWorkspaceActiveSubscriptionSource<'_> {
    async fn session_belongs_to_workspace(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> bool {
        session_belongs_to_workspace(self.state, workspace_id, session_id).await
    }

    async fn active_tasks(
        &self,
        workspace_id: WorkspaceId,
    ) -> Vec<ctx_core::models::WorkspaceActiveTaskSummary> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(workspace_id, i64::MAX)
            .await
            .active
            .tasks
    }

    async fn primary_session_id_for_task(
        &self,
        workspace_id: WorkspaceId,
        task_id: ctx_core::ids::TaskId,
    ) -> Result<Option<SessionId>, ()> {
        let store = self
            .state
            .store_for_workspace(workspace_id)
            .await
            .map_err(|_| ())?;
        let task = store.get_task(task_id).await.map_err(|_| ())?;
        let Some(task) = task else {
            return Ok(None);
        };
        if task.workspace_id != workspace_id {
            return Ok(None);
        }
        Ok(task.primary_session_id)
    }

    async fn session_replay_cursor(
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
}

async fn session_belongs_to_workspace(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
) -> bool {
    let store = match state.store_for_session(session_id).await {
        Ok(store) => store,
        Err(_) => return false,
    };
    match store.get_session(session_id).await {
        Ok(Some(session)) => session.workspace_id == workspace_id,
        _ => false,
    }
}
