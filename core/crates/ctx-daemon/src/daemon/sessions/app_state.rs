use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch, Mutex};

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{Session, SessionEvent};
use ctx_session_runtime::runtime::SessionRuntimeCacheDebugStats;
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::Store;

use crate::daemon::state::DaemonState;

impl DaemonState {
    pub async fn publish_event(self: &Arc<Self>, event: SessionEvent) {
        super::runtime::publish_event(self, event).await;
    }

    pub async fn refresh_session_head_cache(&self, session_id: SessionId) {
        super::runtime::refresh_session_head_cache(self, session_id).await;
    }

    pub async fn task_session_creation_lock(&self, task_id: TaskId) -> Arc<Mutex<()>> {
        self.sessions.task_session_creation_lock(task_id).await
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        self.sessions.remember_session_meta(session).await;
    }

    pub async fn session_order_seq_state(
        &self,
        store: &Store,
        session_id: SessionId,
    ) -> Arc<Mutex<OrderSeqState>> {
        self.sessions.get_order_seq_state(store, session_id).await
    }

    pub async fn is_session_running(&self, session_id: SessionId) -> bool {
        self.sessions.is_running(session_id).await
    }

    pub async fn running_session_ids(&self) -> Vec<SessionId> {
        self.sessions.list_running_sessions().await
    }

    pub async fn session_scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>> {
        self.sessions.scheduler_sender(session_id).await
    }

    pub async fn subscribe_session_event_head(
        &self,
        session_id: SessionId,
    ) -> watch::Receiver<i64> {
        self.sessions.subscribe_session_event_head(session_id).await
    }

    pub async fn provider_inactivity_timeout(&self) -> Duration {
        self.sessions.provider_inactivity_timeout().await
    }

    pub async fn session_cache_debug_stats(&self) -> SessionRuntimeCacheDebugStats {
        self.sessions.cache_debug_stats().await
    }

    pub async fn cached_session_ids_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Vec<SessionId> {
        self.sessions
            .cached_session_ids_for_workspace(workspace_id)
            .await
    }

    pub async fn ensure_scheduler(
        self: &Arc<Self>,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        let state = Arc::downgrade(self);
        self.sessions
            .ensure_scheduler(session, move |session, rx| {
                crate::daemon::scheduler::session_worker(state, session, rx)
            })
            .await
    }
}
