use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch};

use ctx_core::ids::SessionId;
use ctx_core::models::{Session, SessionEvent, SessionHeadSnapshot};

use crate::daemon::state::AppState;

impl AppState {
    pub async fn cached_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Option<SessionHeadSnapshot> {
        self.sessions
            .cached_session_head_snapshot(session_id, limit, include_events)
            .await
    }

    pub async fn cache_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
        snapshot: SessionHeadSnapshot,
    ) {
        self.sessions
            .cache_session_head_snapshot(session_id, limit, include_events, snapshot)
            .await;
    }

    pub async fn get_broadcaster(&self, session_id: SessionId) -> broadcast::Sender<SessionEvent> {
        self.sessions.get_broadcaster(session_id).await
    }

    pub async fn subscribe_session_event_head(
        &self,
        session_id: SessionId,
    ) -> watch::Receiver<i64> {
        self.sessions.subscribe_session_event_head(session_id).await
    }

    pub async fn publish_event(self: &Arc<Self>, event: SessionEvent) {
        self.sessions.publish_event(self, event).await;
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        self.sessions.remember_session_meta(session).await;
    }

    pub async fn refresh_session_head_cache(&self, session_id: SessionId) {
        self.sessions
            .refresh_session_head_cache(self, session_id)
            .await;
    }

    pub async fn ensure_scheduler(
        self: &Arc<Self>,
        session: Session,
    ) -> mpsc::Sender<crate::scheduler::SchedulerCommand> {
        self.sessions.ensure_scheduler(self, session).await
    }

    pub async fn scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<crate::scheduler::SchedulerCommand>> {
        self.sessions.scheduler_sender(session_id).await
    }

    pub async fn is_running(&self, session_id: SessionId) -> bool {
        self.sessions.is_running(session_id).await
    }

    pub async fn list_running_sessions(&self) -> Vec<SessionId> {
        self.sessions.list_running_sessions().await
    }
}
