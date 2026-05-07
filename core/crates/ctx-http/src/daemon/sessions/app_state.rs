use std::sync::Arc;

use tokio::sync::mpsc;

use ctx_core::ids::SessionId;
use ctx_core::models::{Session, SessionEvent};

use crate::daemon::state::AppState;

impl AppState {
    pub async fn publish_event(self: &Arc<Self>, event: SessionEvent) {
        super::runtime::publish_event(self, event).await;
    }

    pub async fn refresh_session_head_cache(&self, session_id: SessionId) {
        super::runtime::refresh_session_head_cache(self, session_id).await;
    }

    pub async fn ensure_scheduler(
        self: &Arc<Self>,
        session: Session,
    ) -> mpsc::Sender<crate::scheduler::SchedulerCommand> {
        let state = Arc::downgrade(self);
        self.sessions
            .ensure_scheduler(session, move |session, rx| {
                crate::scheduler::session_worker(state, session, rx)
            })
            .await
    }
}
