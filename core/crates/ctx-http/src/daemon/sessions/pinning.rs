use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::SessionId;
use ctx_session_service::runtime::SessionLifecycleHost;

use crate::daemon::state::AppState;

impl AppState {
    async fn propagate_provider_session_pin_by_key(&self, session_key: String, pinned: bool) {
        let adapters = {
            let mut adapters = {
                let map = self.providers.adapters.lock().await;
                map.values().cloned().collect::<Vec<_>>()
            };
            let target_adapters = {
                let map = self.providers.target_adapters.lock().await;
                map.values().cloned().collect::<Vec<_>>()
            };
            adapters.extend(target_adapters);
            adapters
        };
        let mut seen = HashSet::<usize>::new();
        for adapter in adapters {
            let identity = (Arc::as_ptr(&adapter) as *const ()) as usize;
            if !seen.insert(identity) {
                continue;
            }
            if let Err(err) = adapter
                .set_session_pinned(session_key.clone(), pinned)
                .await
            {
                tracing::debug!(
                    session_id = %session_key,
                    pinned,
                    err = %err,
                    "failed to update provider worker pin state"
                );
            }
        }
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
impl SessionLifecycleHost for AppState {
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
