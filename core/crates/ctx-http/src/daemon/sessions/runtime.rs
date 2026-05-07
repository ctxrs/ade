use std::sync::Arc;
use std::time::Duration;

use ctx_core::ids::{SessionId, TaskId, TurnId, WorkspaceId};
use ctx_core::models::{
    Session, SessionEvent, SessionHeadDelta, SessionSummaryDelta, SessionTurn,
    SessionTurnToolSummary, TaskDeltaKind,
};

use ctx_session_service::runtime::{
    ActiveTaskRefreshEntry, SessionEventPublicationHost, SessionReplayCursor,
};

use crate::daemon::state::AppState;

const ACTIVE_TASK_REFRESH_DEBOUNCE_MS: u64 = 250;

pub async fn publish_event(state: &Arc<AppState>, event: SessionEvent) {
    state
        .sessions
        .publish_event_with_host(&HttpSessionPublicationHost { state }, event)
        .await;
}

struct HttpSessionPublicationHost<'a> {
    state: &'a Arc<AppState>,
}

#[async_trait::async_trait]
impl SessionEventPublicationHost for HttpSessionPublicationHost<'_> {
    async fn load_session(&self, session_id: SessionId) -> Option<Session> {
        let store = self.state.store_for_session(session_id).await.ok()?;
        store.get_session(session_id).await.ok().flatten()
    }

    async fn list_turn_tool_summaries_for_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Vec<SessionTurnToolSummary> {
        let Ok(store) = self.state.store_for_session(session_id).await else {
            return Vec::new();
        };
        store
            .list_turn_tool_summaries_for_turns(session_id, std::slice::from_ref(&turn_id))
            .await
            .unwrap_or_default()
    }

    async fn cached_turn_for_read(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Option<SessionTurn> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .get_cached_session_head_for_read(session_id)
            .await
            .and_then(|head| head.turns.into_iter().find(|turn| turn.turn_id == turn_id))
    }

    async fn load_turn(&self, session_id: SessionId, turn_id: TurnId) -> Option<SessionTurn> {
        let store = self.state.store_for_session(session_id).await.ok()?;
        store
            .get_session_turn(session_id, turn_id)
            .await
            .ok()
            .flatten()
    }

    async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        let cursor = self
            .state
            .workspaces
            .workspace_active_snapshot
            .session_replay_cursor(workspace_id, session_id)
            .await;
        SessionReplayCursor {
            last_event_seq: cursor.last_event_seq,
            projection_rev: cursor.projection_rev,
        }
    }

    async fn load_projection_rev(&self, session_id: SessionId) -> Option<i64> {
        let store = self.state.store_for_session(session_id).await.ok()?;
        store.get_session_projection_rev(session_id).await.ok()
    }

    async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        session: &Session,
        delta: SessionHeadDelta,
        durable: bool,
    ) {
        self.state
            .workspaces
            .workspace_active_snapshot
            .publish_session_head_delta(workspace_id, session, delta, durable)
            .await;
    }

    async fn publish_session_summary_delta(
        &self,
        workspace_id: WorkspaceId,
        delta: SessionSummaryDelta,
    ) {
        self.state
            .workspaces
            .workspace_active_snapshot
            .publish_session_summary_delta(workspace_id, delta)
            .await;
    }

    async fn queue_task_delta_refresh(&self, task_id: TaskId) {
        queue_workspace_task_delta_refresh(self.state, task_id).await;
    }
}

async fn queue_workspace_task_delta_refresh(state: &Arc<AppState>, task_id: TaskId) {
    let should_spawn = {
        let mut map = state.sessions.active_task_refreshes.lock().await;
        if let Some(entry) = map.get_mut(&task_id) {
            entry.generation = entry.generation.wrapping_add(1);
            false
        } else {
            map.insert(task_id, ActiveTaskRefreshEntry { generation: 1 });
            true
        }
    };
    if should_spawn {
        let state = Arc::downgrade(state);
        tokio::spawn(async move {
            let Some(state) = state.upgrade() else {
                return;
            };
            let state_clone = state.clone();
            run_workspace_task_delta_refresh(state_clone, task_id).await;
        });
    }
}

async fn run_workspace_task_delta_refresh(state: Arc<AppState>, task_id: TaskId) {
    let debounce = Duration::from_millis(ACTIVE_TASK_REFRESH_DEBOUNCE_MS.max(1));
    loop {
        let generation = {
            let map = state.sessions.active_task_refreshes.lock().await;
            match map.get(&task_id) {
                Some(entry) => entry.generation,
                None => return,
            }
        };

        tokio::time::sleep(debounce).await;

        let current = {
            let map = state.sessions.active_task_refreshes.lock().await;
            match map.get(&task_id) {
                Some(entry) => entry.generation,
                None => return,
            }
        };
        if current != generation {
            continue;
        }

        match state.store_for_task(task_id).await {
            Ok(store) => match store.get_workspace_active_task_summary(task_id).await {
                Ok(Some(summary)) => {
                    let _ = state
                        .emit_workspace_task_delta(summary.task, TaskDeltaKind::Updated)
                        .await;
                }
                Ok(None) => match store.get_task(task_id).await {
                    Ok(Some(task)) => {
                        let kind = if task.archived_at.is_some() {
                            TaskDeltaKind::Archived
                        } else {
                            TaskDeltaKind::Updated
                        };
                        let _ = state.emit_workspace_task_delta(task, kind).await;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            task_id = %task_id.0,
                            "workspace task delta refresh read failed: {err:?}"
                        );
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        "workspace task delta refresh summary read failed: {err:?}"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace task delta refresh store lookup failed: {err:?}"
                );
            }
        }

        let mut map = state.sessions.active_task_refreshes.lock().await;
        match map.get(&task_id) {
            Some(entry) if entry.generation == current => {
                map.remove(&task_id);
                return;
            }
            Some(_) => continue,
            None => return,
        }
    }
}

pub async fn refresh_session_head_cache(state: &AppState, session_id: SessionId) {
    let store = match state.store_for_session(session_id).await {
        Ok(store) => store,
        Err(_) => return,
    };
    let head = match store.get_active_snapshot_head(session_id).await {
        Ok(Some(head)) => head,
        Ok(None) => {
            state
                .workspaces
                .workspace_active_snapshot
                .remove_session(session_id)
                .await;
            return;
        }
        Err(err) => {
            tracing::warn!(
                session_id = %session_id.0,
                "active session head cache refresh failed: {err:#}"
            );
            return;
        }
    };
    state
        .workspaces
        .workspace_active_snapshot
        .update_compact_session_head(head)
        .await;
}
