use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use ctx_store::{Store, StoreManager};
use ctx_update_service::UpdateDrainCoordinator;

use super::activity::daemon_turn_activity_summary_parts;

#[derive(Clone)]
pub(in crate::daemon) struct ManagedDaemonAutoUpdateHost {
    data_root: PathBuf,
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
}

impl ManagedDaemonAutoUpdateHost {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
    ) -> Self {
        Self {
            data_root,
            global_store,
            stores,
            update_drain,
        }
    }
}

#[async_trait::async_trait]
impl ctx_update_service::ManagedDaemonAutoUpdateHooks for ManagedDaemonAutoUpdateHost {
    async fn acquire_update_drain(&self, reason: &str, owner: &str) -> bool {
        self.update_drain.acquire(reason, owner).await.is_some()
    }

    async fn release_update_drain(&self) {
        let _ = self.update_drain.release().await;
    }

    async fn daemon_is_idle(&self) -> Result<bool> {
        let activity = daemon_turn_activity_summary_parts(
            &self.global_store,
            &self.stores,
            &self.update_drain,
        )
        .await?;
        Ok(activity.queued_turn_count == 0 && activity.running_turn_count == 0)
    }
}

pub(super) fn spawn_managed_daemon_auto_update(
    host: ManagedDaemonAutoUpdateHost,
    bind: Vec<String>,
) {
    if !ctx_update_service::managed_daemon_auto_update_configured_from_env() {
        return;
    }
    let current_version = match ctx_update_service::current_build_identity(env!(
        "CARGO_PKG_VERSION"
    )) {
        Ok(identity) => identity.exact_version.clone(),
        Err(err) => {
            tracing::warn!(err = %err, "managed daemon auto-update disabled; build identity unavailable");
            return;
        }
    };
    let config = ctx_update_service::ManagedDaemonAutoUpdateConfig {
        data_root: host.data_root.clone(),
        bind,
        current_version,
    };
    let hooks: Arc<dyn ctx_update_service::ManagedDaemonAutoUpdateHooks> = Arc::new(host);
    ctx_update_service::spawn_managed_daemon_auto_update(config, hooks);
}
