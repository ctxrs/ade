use std::collections::{HashMap, HashSet};
use std::future::Future;

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionReplay,
    WorkspaceActiveSnapshotSubscribeScope, WorkspaceActiveTaskSummary,
};

use crate::SessionReplayCursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedWorkspaceActiveSessionReplay {
    Reset,
    Resume {
        after_seq: i64,
        after_projection_rev: i64,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedWorkspaceActiveSessionSubscription {
    pub session_id: SessionId,
    pub replay: ResolvedWorkspaceActiveSessionReplay,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct WorkspaceActiveSubscriptionState {
    pub active_scope: bool,
    pub explicit_sessions: HashSet<SessionId>,
    pub active_task_sessions: HashMap<TaskId, SessionId>,
    pub foreground_session_ids: Option<HashSet<SessionId>>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedWorkspaceActiveSubscriptions {
    pub sessions: Vec<ResolvedWorkspaceActiveSessionSubscription>,
    pub state: WorkspaceActiveSubscriptionState,
}

pub trait WorkspaceActiveSubscriptionSource {
    fn session_belongs_to_workspace(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> impl Future<Output = bool> + Send;

    fn active_tasks(
        &self,
        workspace_id: WorkspaceId,
    ) -> impl Future<Output = Vec<WorkspaceActiveTaskSummary>> + Send;

    fn primary_session_id_for_task(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) -> impl Future<Output = Result<Option<SessionId>, ()>> + Send;

    fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> impl Future<Output = SessionReplayCursor> + Send;
}

pub fn primary_session_id_for_active_task(task: &WorkspaceActiveTaskSummary) -> SessionId {
    task.task
        .primary_session_id
        .unwrap_or(task.primary_session.session.id)
}
pub async fn resolve_workspace_active_snapshot_subscriptions<S>(
    source: &S,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    existing: &HashMap<SessionId, SessionReplayCursor>,
) -> Result<ResolvedWorkspaceActiveSubscriptions, ()>
where
    S: WorkspaceActiveSubscriptionSource + Sync,
{
    let WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids,
        sessions,
        task_ids,
        foreground_session_id,
        scope,
        ..
    } = message;

    let mut resolved = HashSet::new();
    let mut replay_map: HashMap<SessionId, WorkspaceActiveSnapshotSessionReplay> = HashMap::new();
    let mut explicit_sessions = HashSet::new();
    let mut active_task_sessions = HashMap::new();
    let mut active_scope = false;
    let mut foreground_session_ids = None;

    for sub in sessions {
        if !source
            .session_belongs_to_workspace(workspace_id, sub.session_id)
            .await
        {
            continue;
        }
        replay_map.insert(sub.session_id, sub.replay);
        resolved.insert(sub.session_id);
        explicit_sessions.insert(sub.session_id);
    }
    for session_id in session_ids {
        if !source
            .session_belongs_to_workspace(workspace_id, session_id)
            .await
        {
            continue;
        }
        resolved.insert(session_id);
        explicit_sessions.insert(session_id);
    }
    if matches!(scope, Some(WorkspaceActiveSnapshotSubscribeScope::Active)) {
        active_scope = true;
        for task in source.active_tasks(workspace_id).await {
            let session_id = primary_session_id_for_active_task(&task);
            resolved.insert(session_id);
            active_task_sessions.insert(task.task.id, session_id);
        }
    }
    for task_id in task_ids {
        if let Some(primary_session_id) = source
            .primary_session_id_for_task(workspace_id, task_id)
            .await?
        {
            resolved.insert(primary_session_id);
            explicit_sessions.insert(primary_session_id);
        }
    }
    if let Some(session_id) = foreground_session_id {
        if source
            .session_belongs_to_workspace(workspace_id, session_id)
            .await
        {
            let mut sessions = HashSet::new();
            sessions.insert(session_id);
            foreground_session_ids = Some(sessions);
        }
    }

    let mut next = Vec::with_capacity(resolved.len());
    for session_id in resolved {
        let replay = replay_map.get(&session_id);
        let existing_last_sent = existing.get(&session_id).copied();
        let current_tail = if matches!(
            replay,
            Some(WorkspaceActiveSnapshotSessionReplay::Auto) | None
        ) && existing_last_sent.is_none()
        {
            source.session_replay_cursor(workspace_id, session_id).await
        } else {
            SessionReplayCursor::default()
        };
        let replay = resolve_session_replay(replay, existing_last_sent, current_tail);
        next.push(ResolvedWorkspaceActiveSessionSubscription { session_id, replay });
    }
    next.sort_by_key(|subscription| subscription.session_id.0);
    let subscription_state = WorkspaceActiveSubscriptionState {
        active_scope,
        explicit_sessions,
        active_task_sessions,
        foreground_session_ids,
    };
    Ok(ResolvedWorkspaceActiveSubscriptions {
        sessions: next,
        state: subscription_state,
    })
}

pub fn resolve_session_replay(
    replay: Option<&WorkspaceActiveSnapshotSessionReplay>,
    existing_last_sent: Option<SessionReplayCursor>,
    current_tail: SessionReplayCursor,
) -> ResolvedWorkspaceActiveSessionReplay {
    match replay {
        Some(WorkspaceActiveSnapshotSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        }) => ResolvedWorkspaceActiveSessionReplay::Resume {
            after_seq: *after_seq,
            after_projection_rev: *after_projection_rev,
        },
        Some(WorkspaceActiveSnapshotSessionReplay::Reset) => {
            ResolvedWorkspaceActiveSessionReplay::Reset
        }
        Some(WorkspaceActiveSnapshotSessionReplay::Auto) | None => {
            let cursor = existing_last_sent.unwrap_or(current_tail);
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: cursor.last_event_seq,
                after_projection_rev: cursor.projection_rev,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_resolution_uses_existing_cursor_for_auto() {
        let existing_last_sent = Some(SessionReplayCursor {
            last_event_seq: 7,
            projection_rev: 11,
        });
        let current_tail = SessionReplayCursor {
            last_event_seq: 99,
            projection_rev: 101,
        };

        assert_eq!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Auto),
                existing_last_sent,
                current_tail,
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 7,
                after_projection_rev: 11,
            }
        );
        assert_eq!(
            resolve_session_replay(None, existing_last_sent, current_tail),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 7,
                after_projection_rev: 11,
            }
        );
    }

    #[test]
    fn replay_resolution_keeps_reset_explicit_even_with_existing_cursor() {
        assert_eq!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Reset),
                Some(SessionReplayCursor {
                    last_event_seq: 7,
                    projection_rev: 11,
                }),
                SessionReplayCursor {
                    last_event_seq: 12,
                    projection_rev: 18,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Reset
        );
    }

    #[test]
    fn replay_resolution_uses_explicit_resume_cursor() {
        assert_eq!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Resume {
                    after_seq: 21,
                    after_projection_rev: 34,
                }),
                Some(SessionReplayCursor {
                    last_event_seq: 7,
                    projection_rev: 11,
                }),
                SessionReplayCursor {
                    last_event_seq: 12,
                    projection_rev: 18,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 21,
                after_projection_rev: 34,
            }
        );
    }

    #[test]
    fn replay_resolution_uses_current_tail_for_auto_without_existing_cursor() {
        assert_eq!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Auto),
                None,
                SessionReplayCursor {
                    last_event_seq: 12,
                    projection_rev: 18,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 12,
                after_projection_rev: 18,
            }
        );
        assert_eq!(
            resolve_session_replay(
                None,
                None,
                SessionReplayCursor {
                    last_event_seq: 14,
                    projection_rev: 20,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 14,
                after_projection_rev: 20,
            }
        );
    }
}
