use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::SessionId;

use crate::daemon::state::{AppState, SessionPinState, SessionRuntime};

impl SessionRuntime {
    async fn update_pin_state<F>(&self, session_id: SessionId, update: F) -> Option<bool>
    where
        F: FnOnce(&mut SessionPinState),
    {
        let mut pins = self.session_pins.lock().await;
        let entry = pins.entry(session_id).or_default();
        let was_pinned = entry.is_pinned();
        update(entry);
        let is_pinned = entry.is_pinned();
        if !is_pinned {
            pins.remove(&session_id);
        }
        (was_pinned != is_pinned).then_some(is_pinned)
    }

    pub async fn set_running(&self, session_id: SessionId, running: bool) -> Option<bool> {
        let mut set = self.running_sessions.lock().await;
        let changed = if running {
            set.insert(session_id)
        } else {
            set.remove(&session_id)
        };
        drop(set);
        if !changed {
            return None;
        }
        self.update_pin_state(session_id, |state| state.running = running)
            .await
    }

    pub async fn attach_session(&self, session_id: SessionId) -> Option<bool> {
        self.update_pin_state(session_id, |state| {
            state.attached_clients = state.attached_clients.saturating_add(1);
        })
        .await
    }

    pub async fn detach_session(&self, session_id: SessionId) -> Option<bool> {
        self.update_pin_state(session_id, |state| {
            state.attached_clients = state.attached_clients.saturating_sub(1);
        })
        .await
    }

    pub async fn clear_pin_state(&self, session_id: SessionId) -> bool {
        {
            let mut set = self.running_sessions.lock().await;
            set.remove(&session_id);
        }
        let mut pins = self.session_pins.lock().await;
        pins.remove(&session_id)
            .is_some_and(SessionPinState::is_pinned)
    }
}

impl AppState {
    async fn set_provider_session_pinned_by_key(&self, session_key: String, pinned: bool) {
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

    async fn set_provider_session_pinned(&self, session_id: SessionId, pinned: bool) {
        self.set_provider_session_pinned_by_key(session_id.0.to_string(), pinned)
            .await;
    }

    pub async fn set_running(&self, session_id: SessionId, running: bool) {
        if let Some(pinned) = self.sessions.set_running(session_id, running).await {
            self.set_provider_session_pinned(session_id, pinned).await;
        }
    }

    pub async fn attach_session(&self, session_id: SessionId) {
        if let Some(pinned) = self.sessions.attach_session(session_id).await {
            self.set_provider_session_pinned(session_id, pinned).await;
        }
    }

    pub async fn detach_session(&self, session_id: SessionId) {
        if let Some(pinned) = self.sessions.detach_session(session_id).await {
            self.set_provider_session_pinned(session_id, pinned).await;
        }
    }

    pub async fn cleanup_session(&self, session_id: SessionId) {
        if self.sessions.clear_pin_state(session_id).await {
            self.set_provider_session_pinned(session_id, false).await;
        }
        self.sessions.cleanup_session(self, session_id).await;
    }
}
