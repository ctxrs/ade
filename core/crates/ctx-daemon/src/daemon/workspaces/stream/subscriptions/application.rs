use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{TaskDeltaKind, WorkspaceActiveSnapshotEvent};
use ctx_workspace_active_snapshot::{SessionReplayCursor, WorkspaceActiveSubscriptionState};

use crate::daemon::DaemonState;

use super::super::cursor_acceptance::{accept_session_delta_cursor, accept_session_head_cursor};
use super::super::event_routing::{
    plan_workspace_stream_event_route, primary_session_id_for_active_task_event,
    WorkspaceStreamEventRoutePlan,
};
use super::planning::{workspace_stream_session_pin_changes, WorkspaceStreamSessionPinChanges};

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
                    super::super::active_task_subscription_cursor(state, workspace_id, session_id)
                        .await;
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
        state: next_state,
        mut subscriptions,
        pin_changes,
        should_route,
    } = application;
    if !should_route {
        return WorkspaceStreamLiveEventApplication {
            state: next_state,
            subscriptions,
            pin_changes,
            route_plan: WorkspaceStreamEventRoutePlan::Drop,
        };
    }

    let accepted = match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
            let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                return WorkspaceStreamLiveEventApplication {
                    state: next_state,
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
                    state: next_state,
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
        plan_workspace_stream_event_route(&next_state, event)
    } else {
        WorkspaceStreamEventRoutePlan::Drop
    };

    WorkspaceStreamLiveEventApplication {
        state: next_state,
        subscriptions,
        pin_changes,
        route_plan,
    }
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
    let pin_changes =
        workspace_stream_session_pin_changes(previous_subscriptions, next_subscriptions);
    WorkspaceStreamSubscriptionEventApplication {
        state,
        subscriptions,
        pin_changes,
        should_route,
    }
}
