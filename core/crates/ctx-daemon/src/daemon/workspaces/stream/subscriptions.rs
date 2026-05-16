use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionIntent,
};
use ctx_workspace_active_snapshot::{
    resolve_workspace_active_snapshot_subscriptions as resolve_workspace_active_snapshot_subscriptions_with_source,
    ResolvedWorkspaceActiveSessionReplay, ResolvedWorkspaceActiveSessionSubscription,
    ResolvedWorkspaceActiveSubscriptions, SessionReplayCursor, WorkspaceActiveSubscriptionSource,
    WorkspaceActiveSubscriptionState,
};

use crate::daemon::workspaces::WorkspaceHydrationError;
use crate::daemon::DaemonState;

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
