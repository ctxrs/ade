use std::sync::Arc;

use anyhow::Result;

use super::{daemon_turn_activity_summary, AppState};

struct ManagedDaemonAutoUpdateAppHooks {
    state: Arc<AppState>,
}

#[async_trait::async_trait]
impl ctx_update_service::ManagedDaemonAutoUpdateHooks for ManagedDaemonAutoUpdateAppHooks {
    async fn acquire_update_drain(&self, reason: &str, owner: &str) -> bool {
        self.state
            .acquire_update_drain(reason.to_string(), owner.to_string())
            .await
            .is_some()
    }

    async fn release_update_drain(&self) {
        let _ = self.state.release_update_drain().await;
    }

    async fn daemon_is_idle(&self) -> Result<bool> {
        Ok(daemon_turn_activity_summary(&self.state).await?.idle)
    }
}

pub(super) fn spawn_managed_daemon_auto_update(state: Arc<AppState>, bind: Vec<String>) {
    if !ctx_update_service::managed_daemon_auto_update_configured_from_env() {
        return;
    }
    let current_version = match crate::current_build_exact_version() {
        Ok(version) => version,
        Err(err) => {
            tracing::warn!(err = %err, "managed daemon auto-update disabled; build identity unavailable");
            return;
        }
    };
    let config = ctx_update_service::ManagedDaemonAutoUpdateConfig {
        data_root: state.core.data_root.clone(),
        bind,
        current_version,
    };
    let hooks: Arc<dyn ctx_update_service::ManagedDaemonAutoUpdateHooks> =
        Arc::new(ManagedDaemonAutoUpdateAppHooks { state });
    ctx_update_service::spawn_managed_daemon_auto_update(config, hooks);
}
