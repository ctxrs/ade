use ctx_core::ids::SessionId;
use ctx_session_service::runtime::SessionLifecycleHost;

use crate::daemon::state::DaemonState;

impl DaemonState {
    async fn propagate_provider_session_pin_by_key(&self, session_key: String, pinned: bool) {
        self.providers
            .set_provider_session_pinned(session_key, pinned)
            .await;
    }

    async fn propagate_provider_session_pin(&self, session_id: SessionId, pinned: bool) {
        self.propagate_provider_session_pin_by_key(session_id.0.to_string(), pinned)
            .await;
    }

    pub async fn set_running(&self, session_id: SessionId, running: bool) {
        self.sessions
            .set_running_with_host(self, session_id, running)
            .await;
    }

    pub async fn attach_session(&self, session_id: SessionId) {
        self.sessions
            .attach_session_with_host(self, session_id)
            .await;
    }

    pub async fn detach_session(&self, session_id: SessionId) {
        self.sessions
            .detach_session_with_host(self, session_id)
            .await;
    }

    pub async fn cleanup_session(&self, session_id: SessionId) {
        self.sessions
            .cleanup_session_with_host(self, session_id)
            .await;
    }
}

#[async_trait::async_trait]
impl SessionLifecycleHost for DaemonState {
    async fn set_provider_session_pinned(&self, session_id: SessionId, pinned: bool) {
        self.propagate_provider_session_pin(session_id, pinned)
            .await;
    }

    async fn remove_workspace_active_session(&self, session_id: SessionId) {
        let workspace_id = self
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
            .ok()
            .flatten();
        if let Some(workspace_id) = workspace_id {
            self.workspaces
                .workspace_active_snapshot
                .remove_session_with_workspace_hint(workspace_id, session_id)
                .await;
        } else {
            self.workspaces
                .workspace_active_snapshot
                .remove_session(session_id)
                .await;
        }
    }
}
