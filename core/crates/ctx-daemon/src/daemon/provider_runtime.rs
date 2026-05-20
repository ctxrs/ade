use std::path::Path;

use ctx_provider_runtime::ProviderRuntime;
use tokio::sync::broadcast;

use super::DaemonState;

impl ctx_provider_runtime::ProviderRuntimeHost for DaemonState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn current_ctx_version(&self) -> Option<String> {
        match ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION")) {
            Ok(identity) => Some(identity.exact_version.clone()),
            Err(err) => {
                tracing::error!("failed to load ctx build identity for provider runtime: {err:#}");
                None
            }
        }
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        &self.providers
    }

    fn publish_provider_install_ops_events(
        &self,
        events: Vec<ctx_provider_runtime::provider_install_tracker::ProviderInstallOpsEvent>,
    ) {
        self.emit_provider_install_ops_events(events);
    }
}

impl ctx_provider_runtime::provider_usage::ProviderUsageHost for DaemonState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        &self.providers
    }

    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.core.shutdown_tx.subscribe()
    }
}
