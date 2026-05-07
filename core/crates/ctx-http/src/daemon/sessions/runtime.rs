use std::sync::{Arc, Weak};

use ctx_core::ids::{SessionId, TaskId, TurnId, WorkspaceId};
use ctx_core::models::{
    Session, SessionEvent, SessionHeadDelta, SessionHeadSnapshot, SessionSummaryDelta, SessionTurn,
    SessionTurnToolSummary, TaskDeltaKind,
};

use ctx_session_service::runtime::{
    SessionEventPublicationHost, SessionHeadRefreshHost, SessionHeadRefreshLoad,
    SessionReplayCursor, SessionTaskDeltaRefreshHost,
};

use crate::daemon::state::AppState;

pub async fn publish_event(state: &Arc<AppState>, event: SessionEvent) {
    let task_delta_refresh_host = Arc::new(HttpTaskDeltaRefreshHost {
        state: Arc::downgrade(state),
    });
    let host = HttpSessionPublicationHost {
        state: Arc::clone(state),
        task_delta_refresh_host,
    };
    state.sessions.publish_event_with_host(&host, event).await;
}

struct HttpSessionPublicationHost {
    state: Arc<AppState>,
    task_delta_refresh_host: Arc<HttpTaskDeltaRefreshHost>,
}

struct HttpTaskDeltaRefreshHost {
    state: Weak<AppState>,
}

#[async_trait::async_trait]
impl SessionEventPublicationHost for HttpSessionPublicationHost {
    type TaskDeltaRefreshHost = HttpTaskDeltaRefreshHost;

    fn task_delta_refresh_host(&self) -> Arc<Self::TaskDeltaRefreshHost> {
        Arc::clone(&self.task_delta_refresh_host)
    }

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
}

#[async_trait::async_trait]
impl SessionTaskDeltaRefreshHost for HttpTaskDeltaRefreshHost {
    async fn emit_task_delta_refresh(&self, task_id: TaskId) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
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
    }
}

pub async fn refresh_session_head_cache(state: &AppState, session_id: SessionId) {
    state
        .sessions
        .refresh_session_head_cache_with_host(state, session_id)
        .await;
}

#[async_trait::async_trait]
impl SessionHeadRefreshHost for AppState {
    async fn load_active_snapshot_head(&self, session_id: SessionId) -> SessionHeadRefreshLoad {
        let store = match self.store_for_session(session_id).await {
            Ok(store) => store,
            Err(err) => {
                return SessionHeadRefreshLoad::Failed {
                    error: format!("{err:#}"),
                };
            }
        };
        match store.get_active_snapshot_head(session_id).await {
            Ok(Some(head)) => SessionHeadRefreshLoad::Found(head),
            Ok(None) => SessionHeadRefreshLoad::Missing,
            Err(err) => SessionHeadRefreshLoad::Failed {
                error: format!("{err:#}"),
            },
        }
    }

    async fn update_compact_session_head(&self, head: SessionHeadSnapshot) {
        self.workspaces
            .workspace_active_snapshot
            .update_compact_session_head(head)
            .await;
    }

    async fn remove_session_from_active_head_cache(&self, session_id: SessionId) {
        self.workspaces
            .workspace_active_snapshot
            .remove_session(session_id)
            .await;
    }
}
