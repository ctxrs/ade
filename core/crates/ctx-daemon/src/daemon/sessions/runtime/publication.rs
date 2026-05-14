use std::sync::Arc;

use ctx_core::ids::{SessionId, TurnId, WorkspaceId};
use ctx_core::models::{
    Session, SessionEvent, SessionHeadDelta, SessionSummaryDelta, SessionTurn,
    SessionTurnToolSummary,
};
use ctx_session_service::runtime::{SessionEventPublicationHost, SessionReplayCursor};

use crate::daemon::state::DaemonState;

mod task_delta;
use task_delta::HttpTaskDeltaRefreshHost;

pub async fn publish_event(state: &Arc<DaemonState>, event: SessionEvent) {
    let task_delta_refresh_host = Arc::new(HttpTaskDeltaRefreshHost::new(state));
    let host = HttpSessionPublicationHost {
        state: Arc::clone(state),
        task_delta_refresh_host,
    };
    state.sessions.publish_event_with_host(&host, event).await;
}

struct HttpSessionPublicationHost {
    state: Arc<DaemonState>,
    task_delta_refresh_host: Arc<HttpTaskDeltaRefreshHost>,
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
