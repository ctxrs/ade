use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use tokio::sync::Mutex;

use crate::daemon::AppState;

impl ctx_provider_runtime::ProviderRuntimeHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
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
