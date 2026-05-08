use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotStreamMessage, Worktree, WorktreeVcsFreshness,
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

pub(crate) async fn refresh_worktree_vcs_for_worktrees(
    state: &Arc<AppState>,
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
        let should_refresh = match state.get_worktree_vcs_snapshot(worktree.id).await {
            Some(snapshot)
                if snapshot.freshness == WorktreeVcsFreshness::Fresh
                    && snapshot.available
                    && (!details
                        || matches!(
                            snapshot.touched_files_state,
                            ctx_core::models::WorktreeVcsTouchedFilesState::Ready
                        )) =>
            {
                false
            }
            _ => true,
        };
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

async fn load_worktree(state: &Arc<AppState>, worktree_id: WorktreeId) -> Option<Worktree> {
    let store = state.store_for_worktree(worktree_id).await.ok()?;
    store.get_worktree(worktree_id).await.ok().flatten()
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

#[cfg(test)]
mod tests;
