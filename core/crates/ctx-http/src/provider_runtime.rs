use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use tokio::sync::{broadcast, Mutex};

use crate::daemon::AppState;

impl ctx_provider_runtime::ProviderRuntimeHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn current_ctx_version(&self) -> Option<String> {
        match crate::current_build_identity() {
            Ok(identity) => Some(identity.exact_version.clone()),
            Err(err) => {
                tracing::error!("failed to load ctx build identity for provider runtime: {err:#}");
                None
            }
        }
    }

    fn provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>> {
        &self.providers.adapters
    }

    fn target_provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>> {
        &self.providers.target_adapters
    }

    fn provider_statuses(&self) -> &Mutex<HashMap<String, ProviderStatus>> {
        &self.providers.statuses
    }
}

impl ctx_provider_runtime::provider_usage::ProviderUsageHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn usage_cache(
        &self,
    ) -> &Mutex<HashMap<String, ctx_provider_runtime::provider_usage::ProviderUsageSnapshot>> {
        &self.providers.usage_cache
    }

    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.core.shutdown_tx.subscribe()
    }
}
