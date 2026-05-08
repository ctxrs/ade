use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotStreamMessage, Worktree, WorktreeVcsFreshness, WorktreeVcsSnapshot,
};
use ctx_workspace_active_snapshot::{
    resolve_workspace_active_snapshot_subscriptions as resolve_workspace_active_snapshot_subscriptions_with_source,
    ResolvedWorkspaceActiveSubscriptions, SessionReplayCursor, WorkspaceActiveSubscriptionSource,
    WorkspaceSessionReplay, WorkspaceSessionReplayItem,
};

use crate::daemon::AppState;

pub(crate) enum ReplayOutcome {
    Replay { last_sent: SessionReplayCursor },
    ResetRequired,
}

const SESSION_REPLAY_MAX_EVENTS: usize = 2000;

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
    resolve_workspace_active_snapshot_subscriptions_with_source(
        &HttpWorkspaceActiveSubscriptionSource { state },
        workspace_id,
        message,
        existing,
    )
    .await
}

struct HttpWorkspaceActiveSubscriptionSource<'a> {
    state: &'a Arc<AppState>,
}

impl WorkspaceActiveSubscriptionSource for HttpWorkspaceActiveSubscriptionSource<'_> {
    fn session_belongs_to_workspace(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> impl std::future::Future<Output = bool> + Send {
        async move { session_belongs_to_workspace(self.state, workspace_id, session_id).await }
    }

    fn active_tasks(
        &self,
        workspace_id: WorkspaceId,
    ) -> impl std::future::Future<Output = Vec<ctx_core::models::WorkspaceActiveTaskSummary>> + Send
    {
        async move {
            self.state
                .workspaces
                .workspace_active_snapshot
                .active_snapshot(workspace_id, i64::MAX)
                .await
                .active
                .tasks
        }
    }

    fn primary_session_id_for_task(
        &self,
        workspace_id: WorkspaceId,
        task_id: ctx_core::ids::TaskId,
    ) -> impl std::future::Future<Output = Result<Option<SessionId>, ()>> + Send {
        async move {
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
    }

    fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> impl std::future::Future<Output = SessionReplayCursor> + Send {
        async move {
            self.state
                .workspaces
                .workspace_active_snapshot
                .session_replay_cursor(workspace_id, session_id)
                .await
        }
    }

    fn worktree_vcs_enabled(&self) -> bool {
        self.state.worktree_vcs_enabled()
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
                // Subscription warm-up should not downgrade an already-published ready snapshot.
                // Real filesystem invalidations still use the transient stale path.
                if let Err(err) =
                    crate::daemon::git_status::request_worktree_vcs_refresh_without_transient(
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
                if let Err(err) =
                    crate::daemon::git_status::request_worktree_vcs_refresh_without_transient(
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
    worktree_ids.extend(resolve_worktree_ids_for_sessions(state, open_session_ids).await);
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
mod tests;
