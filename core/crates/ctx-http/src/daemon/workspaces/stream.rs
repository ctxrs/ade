use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotSessionReplay, WorkspaceActiveSnapshotStreamMessage,
    WorkspaceActiveSnapshotSubscribeScope, WorkspaceActiveTaskSummary, Worktree,
    WorktreeVcsFreshness, WorktreeVcsSnapshot,
};
use ctx_workspace_active_snapshot::{
    SessionReplayCursor, WorkspaceSessionReplay, WorkspaceSessionReplayItem,
};

use crate::daemon::AppState;

#[derive(Clone, Copy)]
pub(crate) enum ResolvedWorkspaceActiveSessionReplay {
    Reset,
    Resume {
        after_seq: i64,
        after_projection_rev: i64,
    },
}

pub(crate) struct ResolvedWorkspaceActiveSessionSubscription {
    pub(crate) session_id: SessionId,
    pub(crate) replay: ResolvedWorkspaceActiveSessionReplay,
}

#[derive(Default)]
pub(crate) struct WorkspaceActiveSubscriptionState {
    pub(crate) active_scope: bool,
    pub(crate) explicit_sessions: HashSet<SessionId>,
    pub(crate) active_task_sessions: HashMap<TaskId, SessionId>,
    pub(crate) active_task_vcs_sessions: HashMap<TaskId, HashSet<SessionId>>,
    pub(crate) vcs_open_sessions: HashSet<SessionId>,
    pub(crate) foreground_session_ids: Option<HashSet<SessionId>>,
}

pub(crate) struct ResolvedWorkspaceActiveSubscriptions {
    pub(crate) sessions: Vec<ResolvedWorkspaceActiveSessionSubscription>,
    pub(crate) worktree_vcs_summary_session_ids: Vec<SessionId>,
    pub(crate) worktree_vcs_open_session_ids: Vec<SessionId>,
    pub(crate) state: WorkspaceActiveSubscriptionState,
}

pub(crate) enum ReplayOutcome {
    Replay { last_sent: SessionReplayCursor },
    ResetRequired,
}

const SESSION_REPLAY_MAX_EVENTS: usize = 2000;

pub(crate) fn primary_session_id_for_active_task(task: &WorkspaceActiveTaskSummary) -> SessionId {
    task.task
        .primary_session_id
        .unwrap_or(task.primary_session.session.id)
}

pub(crate) fn primary_session_ids_for_active_task_summary(
    task: &WorkspaceActiveTaskSummary,
) -> HashSet<SessionId> {
    let mut sessions = HashSet::new();
    sessions.insert(primary_session_id_for_active_task(task));
    sessions
}

pub(crate) fn resolve_worktree_vcs_summary_session_ids<I>(
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

pub(crate) fn resolve_worktree_vcs_open_session_ids(
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

pub(crate) async fn replay_session_events<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_cursor: SessionReplayCursor,
    list_failpoint: &'static str,
    send_failpoint: Option<&'static str>,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) =
        crate::api::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail(list_failpoint).is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
        .workspaces
        .workspace_active_snapshot
        .replay_session_stream(
            workspace_id,
            session_id,
            after_cursor.last_event_seq,
            after_cursor.projection_rev,
            SESSION_REPLAY_MAX_EVENTS,
        )
        .await;
    match replay {
        WorkspaceSessionReplay::Replay {
            mut items,
            mut last_sent,
        } => {
            let saw_gap = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Gap { .. }));
            let saw_seed = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Seed(_)));
            if saw_gap && !saw_seed {
                let store = state.store_for_session(session_id).await.map_err(|_| ())?;
                if let Ok(Some(head)) = store.get_session_head_snapshot(session_id, 60, true).await
                {
                    state
                        .workspaces
                        .workspace_active_snapshot
                        .update_session_head(head.clone())
                        .await;
                    last_sent = SessionReplayCursor::from_head(&head);
                    items.push(WorkspaceSessionReplayItem::Seed(Box::new(head)));
                }
            }
            for item in items {
                if matches!(item, WorkspaceSessionReplayItem::Delta(_)) {
                    if let Some(label) = send_failpoint {
                        crate::fault_injection::maybe_fail(label).map_err(|_| ())?;
                    }
                }
                let event = match item {
                    WorkspaceSessionReplayItem::Delta(delta) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            workspace_id,
                            snapshot_rev,
                            delta,
                        }
                    }
                    WorkspaceSessionReplayItem::Gap {
                        session_id,
                        after_seq,
                        reason,
                    } => WorkspaceActiveSnapshotEvent::SessionGap {
                        workspace_id,
                        snapshot_rev,
                        session_id,
                        after_seq,
                        reason,
                    },
                    WorkspaceSessionReplayItem::Seed(head) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                            workspace_id,
                            snapshot_rev,
                            head,
                        }
                    }
                };
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(event),
                })
                .await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        WorkspaceSessionReplay::ResetRequired => Ok(ReplayOutcome::ResetRequired),
    }
}

pub(crate) async fn resolve_workspace_active_snapshot_subscriptions(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    existing: &HashMap<SessionId, SessionReplayCursor>,
) -> Result<ResolvedWorkspaceActiveSubscriptions, ()> {
    match message {
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids,
            sessions,
            task_ids,
            foreground_session_id,
            scope,
            vcs_open_session_ids,
            ..
        } => {
            let mut resolved = HashSet::new();
            let mut replay_map: HashMap<SessionId, WorkspaceActiveSnapshotSessionReplay> =
                HashMap::new();
            let mut explicit_sessions = HashSet::new();
            let mut active_task_sessions = HashMap::new();
            let mut active_task_vcs_sessions = HashMap::new();
            let mut open_vcs_sessions = HashSet::new();
            let mut active_scope = false;
            let mut foreground_session_ids = None;
            for sub in sessions {
                if !session_belongs_to_workspace(state, workspace_id, sub.session_id).await {
                    continue;
                }
                replay_map.insert(sub.session_id, sub.replay);
                resolved.insert(sub.session_id);
                explicit_sessions.insert(sub.session_id);
            }
            for session_id in session_ids {
                if !session_belongs_to_workspace(state, workspace_id, session_id).await {
                    continue;
                }
                resolved.insert(session_id);
                explicit_sessions.insert(session_id);
            }
            if matches!(scope, Some(WorkspaceActiveSnapshotSubscribeScope::Active)) {
                active_scope = true;
                let snapshot = state
                    .workspaces
                    .workspace_active_snapshot
                    .active_snapshot(workspace_id, i64::MAX)
                    .await;
                for task in snapshot.active.tasks {
                    let session_id = primary_session_id_for_active_task(&task);
                    resolved.insert(session_id);
                    active_task_sessions.insert(task.task.id, session_id);
                    active_task_vcs_sessions.insert(
                        task.task.id,
                        primary_session_ids_for_active_task_summary(&task),
                    );
                }
            }
            if !task_ids.is_empty() {
                let store = state
                    .store_for_workspace(workspace_id)
                    .await
                    .map_err(|_| ())?;
                for task_id in task_ids {
                    let task = store.get_task(task_id).await.map_err(|_| ())?;
                    let Some(task) = task else {
                        continue;
                    };
                    if task.workspace_id != workspace_id {
                        continue;
                    }
                    if let Some(primary_session_id) = task.primary_session_id {
                        resolved.insert(primary_session_id);
                        explicit_sessions.insert(primary_session_id);
                    }
                }
            }
            for session_id in vcs_open_session_ids {
                if !session_belongs_to_workspace(state, workspace_id, session_id).await {
                    continue;
                }
                open_vcs_sessions.insert(session_id);
                resolved.insert(session_id);
            }
            if let Some(session_id) = foreground_session_id {
                if session_belongs_to_workspace(state, workspace_id, session_id).await {
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
                    state
                        .workspaces
                        .workspace_active_snapshot
                        .session_replay_cursor(workspace_id, session_id)
                        .await
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
            let worktree_vcs_open_session_ids =
                resolve_worktree_vcs_open_session_ids(&subscription_state);
            let (worktree_vcs_summary_session_ids, worktree_vcs_open_session_ids) =
                if state.worktree_vcs_enabled() {
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
    }
}

async fn session_belongs_to_workspace(
    state: &Arc<AppState>,
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

pub(crate) async fn refresh_worktree_vcs_for_sessions(
    state: &Arc<AppState>,
    summary_session_ids: &[SessionId],
    open_session_ids: &[SessionId],
) {
    if !state.worktree_vcs_enabled() {
        return;
    }
    if summary_session_ids.is_empty() && open_session_ids.is_empty() {
        return;
    }
    let mut worktrees: HashMap<WorktreeId, (Worktree, bool)> = HashMap::new();
    for session_id in summary_session_ids {
        let store = match state.store_for_session(*session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = match store.get_session(*session_id).await {
            Ok(Some(session)) => session,
            _ => continue,
        };
        let worktree = match store.get_worktree(session.worktree_id).await {
            Ok(Some(worktree)) => worktree,
            _ => continue,
        };
        worktrees.entry(worktree.id).or_insert((worktree, false));
    }
    for session_id in open_session_ids {
        let store = match state.store_for_session(*session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = match store.get_session(*session_id).await {
            Ok(Some(session)) => session,
            _ => continue,
        };
        let worktree = match store.get_worktree(session.worktree_id).await {
            Ok(Some(worktree)) => worktree,
            _ => continue,
        };
        worktrees
            .entry(worktree.id)
            .and_modify(|(_, open)| *open = true)
            .or_insert((worktree, true));
    }

    for (worktree_id, (worktree, open_pane)) in worktrees {
        state.ensure_git_status_watcher(worktree.clone()).await;
        match state.get_worktree_vcs_snapshot(worktree.id).await {
            Some(snapshot)
                if snapshot.freshness == WorktreeVcsFreshness::Fresh
                    && snapshot.available
                    && (!open_pane
                        || matches!(
                            snapshot.touched_files_state,
                            ctx_core::models::WorktreeVcsTouchedFilesState::Ready
                        )) => {}
            Some(_) => {
                if let Err(err) = crate::git_status::request_worktree_vcs_refresh(
                    state, &worktree, true, open_pane,
                )
                .await
                {
                    tracing::warn!(
                        worktree_id = %worktree_id.0,
                        "worktree vcs refresh failed: {err:#}"
                    );
                }
            }
            None => {
                if let Err(err) = crate::git_status::request_worktree_vcs_refresh(
                    state, &worktree, true, open_pane,
                )
                .await
                {
                    tracing::warn!(
                        worktree_id = %worktree_id.0,
                        "worktree vcs seed failed: {err:#}"
                    );
                }
            }
        }
    }
}

pub(crate) fn spawn_worktree_vcs_refresh_for_sessions(
    state: Arc<AppState>,
    summary_session_ids: Vec<SessionId>,
    open_session_ids: Vec<SessionId>,
) {
    if summary_session_ids.is_empty() && open_session_ids.is_empty() {
        return;
    }
    tokio::spawn(async move {
        refresh_worktree_vcs_for_sessions(&state, &summary_session_ids, &open_session_ids).await;
    });
}

pub(crate) async fn load_worktree_vcs_snapshots_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
) -> Vec<WorktreeVcsSnapshot> {
    if !state.worktree_vcs_enabled() {
        return Vec::new();
    }
    let worktree_ids = resolve_worktree_ids_for_sessions(state, session_ids).await;
    let mut ordered_worktree_ids: Vec<_> = worktree_ids.into_iter().collect();
    ordered_worktree_ids.sort_by_key(|worktree_id| worktree_id.0);
    let mut snapshots = Vec::new();
    for worktree_id in ordered_worktree_ids {
        if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree_id).await {
            snapshots.push(snapshot);
        }
    }
    snapshots
}

pub(crate) async fn resolve_worktree_vcs_publish_worktree_ids(
    state: &Arc<AppState>,
    summary_session_ids: &[SessionId],
    open_session_ids: &[SessionId],
) -> HashSet<WorktreeId> {
    if !state.worktree_vcs_enabled() {
        return HashSet::new();
    }
    let mut worktree_ids = resolve_worktree_ids_for_sessions(state, summary_session_ids).await;
    worktree_ids.extend(
        resolve_worktree_ids_for_sessions(state, open_session_ids)
            .await
            .into_iter(),
    );
    worktree_ids
}

pub(crate) async fn sync_active_worktrees(
    state: &Arc<AppState>,
    active_worktrees: &mut HashSet<WorktreeId>,
    open_worktrees: &mut HashSet<WorktreeId>,
    summary_session_ids: &[SessionId],
    open_session_ids: &[SessionId],
) {
    if !state.worktree_vcs_enabled() {
        state
            .update_worktree_vcs_activity(active_worktrees, &HashSet::new())
            .await;
        state
            .update_worktree_vcs_open_panes(open_worktrees, &HashSet::new())
            .await;
        active_worktrees.clear();
        open_worktrees.clear();
        return;
    }
    let mut next = resolve_worktree_ids_for_sessions(state, summary_session_ids).await;
    let next_open = resolve_worktree_ids_for_sessions(state, open_session_ids).await;
    next.extend(next_open.iter().copied());
    state
        .update_worktree_vcs_activity(active_worktrees, &next)
        .await;
    state
        .update_worktree_vcs_open_panes(open_worktrees, &next_open)
        .await;
    *active_worktrees = next;
    *open_worktrees = next_open;
}

pub(crate) fn merge_worktree_vcs_snapshots(
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

fn resolve_session_replay(
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

async fn resolve_worktree_ids_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
) -> HashSet<WorktreeId> {
    let mut worktree_ids = HashSet::new();
    for session_id in session_ids {
        let store = match state.store_for_session(*session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = match store.get_session(*session_id).await {
            Ok(Some(session)) => session,
            _ => continue,
        };
        worktree_ids.insert(session.worktree_id);
    }
    worktree_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use ctx_core::models::ExecutionEnvironment;
    use ctx_store::StoreManager;

    fn session_id(value: &str) -> SessionId {
        SessionId(uuid::Uuid::parse_str(value).unwrap())
    }

    async fn create_workspace_session(
        state: &Arc<AppState>,
        root: &Path,
    ) -> (WorkspaceId, SessionId) {
        let workspace = state
            .global_store()
            .create_workspace(
                format!("ws-{}", uuid::Uuid::new_v4()),
                root.join(format!("ws-{}", uuid::Uuid::new_v4()))
                    .to_string_lossy()
                    .to_string(),
                ctx_core::models::VcsKind::Git,
            )
            .await
            .unwrap();
        let store = state.store_for_workspace(workspace.id).await.unwrap();
        let worktree = store
            .create_worktree(
                workspace.id,
                root.join(format!("worktree-{}", uuid::Uuid::new_v4()))
                    .to_string_lossy()
                    .to_string(),
                "deadbeef".to_string(),
                None,
            )
            .await
            .unwrap();
        let task = store
            .create_task(workspace.id, "task".to_string(), None)
            .await
            .unwrap();
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                ExecutionEnvironment::Host,
                "fake".to_string(),
                "model".to_string(),
                "implementer".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await
            .unwrap();
        (workspace.id, session.id)
    }

    #[test]
    fn replay_deserializes_subscribe_message_with_explicit_replay_modes() {
        let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
            r#"{
                "type":"subscribe",
                "sessions":[
                    {
                        "session_id":"00000000-0000-0000-0000-000000000001",
                        "replay":{"mode":"auto"}
                    },
                    {
                        "session_id":"00000000-0000-0000-0000-000000000002",
                        "replay":{"mode":"resume","after_seq":12}
                    },
                    {
                        "session_id":"00000000-0000-0000-0000-000000000003",
                        "replay":{"mode":"reset"}
                    }
                ]
            }"#,
        )
        .unwrap();

        let WorkspaceActiveSnapshotClientMessage::Subscribe { sessions, .. } = message;
        assert_eq!(sessions.len(), 3);
        assert!(matches!(
            sessions[0].replay,
            WorkspaceActiveSnapshotSessionReplay::Auto
        ));
        assert!(matches!(
            sessions[1].replay,
            WorkspaceActiveSnapshotSessionReplay::Resume {
                after_seq: 12,
                after_projection_rev: 0,
            }
        ));
        assert!(matches!(
            sessions[2].replay,
            WorkspaceActiveSnapshotSessionReplay::Reset
        ));
    }

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

        assert!(matches!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Auto),
                existing_last_sent,
                current_tail,
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 7,
                after_projection_rev: 11,
            }
        ));
        assert!(matches!(
            resolve_session_replay(None, existing_last_sent, current_tail),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 7,
                after_projection_rev: 11,
            }
        ));
    }

    #[test]
    fn replay_resolution_keeps_reset_explicit_even_with_existing_cursor() {
        assert!(matches!(
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
        ));
    }

    #[test]
    fn replay_resolution_uses_explicit_resume_cursor() {
        assert!(matches!(
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
        ));
    }

    #[test]
    fn replay_resolution_uses_current_tail_for_auto_without_existing_cursor() {
        assert!(matches!(
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
        ));
        assert!(matches!(
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
        ));
    }

    #[test]
    fn replay_deserialization_keeps_session_ids_and_explicit_sessions_distinct() {
        let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
            r#"{
                "type":"subscribe",
                "session_ids":[
                    "00000000-0000-0000-0000-000000000001",
                    "00000000-0000-0000-0000-000000000002"
                ],
                "sessions":[
                    {
                        "session_id":"00000000-0000-0000-0000-000000000003",
                        "replay":{"mode":"reset"}
                    }
                ]
            }"#,
        )
        .unwrap();

        let WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids,
            sessions,
            ..
        } = message;
        assert_eq!(
            session_ids,
            vec![
                session_id("00000000-0000-0000-0000-000000000001"),
                session_id("00000000-0000-0000-0000-000000000002"),
            ]
        );
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].session_id,
            session_id("00000000-0000-0000-0000-000000000003")
        );
        assert!(matches!(
            sessions[0].replay,
            WorkspaceActiveSnapshotSessionReplay::Reset
        ));
    }

    #[test]
    fn replay_deserialization_reads_foreground_session_id() {
        let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
            r#"{
                "type":"subscribe",
                "foreground_session_id":"00000000-0000-0000-0000-000000000001"
            }"#,
        )
        .unwrap();

        let WorkspaceActiveSnapshotClientMessage::Subscribe {
            foreground_session_id,
            ..
        } = message;
        assert_eq!(
            foreground_session_id,
            Some(session_id("00000000-0000-0000-0000-000000000001"))
        );
    }

    #[tokio::test]
    async fn subscription_resolution_filters_cross_workspace_session_references() {
        let root = tempfile::tempdir().unwrap();
        let stores = StoreManager::open(root.path()).await.unwrap();
        let state = Arc::new(AppState::new(
            root.path().to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:4399".to_string(),
            Some("daemon-secret".to_string()),
        ));
        let (workspace_a, session_a) = create_workspace_session(&state, root.path()).await;
        let (_workspace_b, session_b) = create_workspace_session(&state, root.path()).await;

        let resolved = resolve_workspace_active_snapshot_subscriptions(
            &state,
            workspace_a,
            WorkspaceActiveSnapshotClientMessage::Subscribe {
                session_ids: vec![session_a, session_b],
                sessions: vec![
                    ctx_core::models::WorkspaceActiveSnapshotSessionSubscription {
                        session_id: session_b,
                        replay: WorkspaceActiveSnapshotSessionReplay::Reset,
                    },
                ],
                task_ids: Vec::new(),
                foreground_session_id: Some(session_b),
                vcs_open_session_ids: vec![session_b],
                scope: None,
                include_active_heads: false,
            },
            &HashMap::new(),
        )
        .await
        .unwrap();

        assert_eq!(resolved.sessions.len(), 1);
        assert_eq!(resolved.sessions[0].session_id, session_a);
        assert_eq!(resolved.worktree_vcs_summary_session_ids, vec![session_a]);
        assert!(resolved.worktree_vcs_open_session_ids.is_empty());
        assert!(resolved.state.vcs_open_sessions.is_empty());
        assert!(resolved.state.foreground_session_ids.is_none());
        assert_eq!(resolved.state.explicit_sessions, HashSet::from([session_a]));
    }
}
