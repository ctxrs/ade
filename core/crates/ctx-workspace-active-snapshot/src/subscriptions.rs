use std::collections::{HashMap, HashSet};
use std::future::Future;

use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionReplay,
    WorkspaceActiveSnapshotSubscribeScope, WorkspaceActiveTaskSummary, WorktreeVcsSnapshot,
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
    pub active_task_vcs_sessions: HashMap<TaskId, HashSet<SessionId>>,
    pub vcs_open_sessions: HashSet<SessionId>,
    pub foreground_session_ids: Option<HashSet<SessionId>>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedWorkspaceActiveSubscriptions {
    pub sessions: Vec<ResolvedWorkspaceActiveSessionSubscription>,
    pub worktree_vcs_summary_session_ids: Vec<SessionId>,
    pub worktree_vcs_open_session_ids: Vec<SessionId>,
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

    fn worktree_vcs_enabled(&self) -> bool;
}

pub fn primary_session_id_for_active_task(task: &WorkspaceActiveTaskSummary) -> SessionId {
    task.task
        .primary_session_id
        .unwrap_or(task.primary_session.session.id)
}

pub fn primary_session_ids_for_active_task_summary(
    task: &WorkspaceActiveTaskSummary,
) -> HashSet<SessionId> {
    let mut sessions = HashSet::new();
    sessions.insert(primary_session_id_for_active_task(task));
    sessions
}

pub fn resolve_worktree_vcs_summary_session_ids<I>(
    session_ids: I,
    subscription_state: &WorkspaceActiveSubscriptionState,
) -> Vec<SessionId>
where
    I: IntoIterator<Item = SessionId>,
{
    let mut ids: HashSet<SessionId> = session_ids.into_iter().collect();
    for task_session_ids in subscription_state.active_task_vcs_sessions.values() {
        ids.extend(task_session_ids.iter().copied());
    }
    let mut ordered: Vec<_> = ids.into_iter().collect();
    ordered.sort_by_key(|session_id| session_id.0);
    ordered
}

pub fn resolve_worktree_vcs_open_session_ids(
    subscription_state: &WorkspaceActiveSubscriptionState,
) -> Vec<SessionId> {
    let mut ordered: Vec<_> = subscription_state
        .vcs_open_sessions
        .iter()
        .copied()
        .collect();
    ordered.sort_by_key(|session_id| session_id.0);
    ordered
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
        vcs_open_session_ids,
        ..
    } = message;

    let mut resolved = HashSet::new();
    let mut replay_map: HashMap<SessionId, WorkspaceActiveSnapshotSessionReplay> = HashMap::new();
    let mut explicit_sessions = HashSet::new();
    let mut active_task_sessions = HashMap::new();
    let mut active_task_vcs_sessions = HashMap::new();
    let mut open_vcs_sessions = HashSet::new();
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
            active_task_vcs_sessions.insert(
                task.task.id,
                primary_session_ids_for_active_task_summary(&task),
            );
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
    for session_id in vcs_open_session_ids {
        if !source
            .session_belongs_to_workspace(workspace_id, session_id)
            .await
        {
            continue;
        }
        open_vcs_sessions.insert(session_id);
        resolved.insert(session_id);
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
        active_task_vcs_sessions,
        vcs_open_sessions: open_vcs_sessions,
        foreground_session_ids,
    };
    let worktree_vcs_summary_session_ids = resolve_worktree_vcs_summary_session_ids(
        next.iter().map(|sub| sub.session_id),
        &subscription_state,
    );
    let worktree_vcs_open_session_ids = resolve_worktree_vcs_open_session_ids(&subscription_state);
    let (worktree_vcs_summary_session_ids, worktree_vcs_open_session_ids) =
        if source.worktree_vcs_enabled() {
            (
                worktree_vcs_summary_session_ids,
                worktree_vcs_open_session_ids,
            )
        } else {
            (Vec::new(), Vec::new())
        };
    Ok(ResolvedWorkspaceActiveSubscriptions {
        sessions: next,
        worktree_vcs_summary_session_ids,
        worktree_vcs_open_session_ids,
        state: subscription_state,
    })
}

pub fn merge_worktree_vcs_snapshots(
    snapshots: Vec<WorktreeVcsSnapshot>,
    extras: Vec<WorktreeVcsSnapshot>,
) -> Vec<WorktreeVcsSnapshot> {
    let mut merged: HashMap<WorktreeId, WorktreeVcsSnapshot> = HashMap::new();
    for snapshot in snapshots {
        merged.insert(snapshot.worktree_id, snapshot);
    }
    for snapshot in extras {
        merged.insert(snapshot.worktree_id, snapshot);
    }
    let mut ordered: Vec<_> = merged.into_values().collect();
    ordered.sort_by_key(|snapshot| snapshot.worktree_id.0);
    ordered
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
