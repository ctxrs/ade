use std::collections::HashMap;
use std::sync::Arc;

use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};

use crate::ProviderRuntime;

impl ProviderRuntime {
    pub async fn with_provider_adapters<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, Arc<dyn ProviderAdapter>>) -> R,
    ) -> R {
        let mut adapters = self.adapters.lock().await;
        f(&mut adapters)
    }

    pub async fn with_target_provider_adapters<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, Arc<dyn ProviderAdapter>>) -> R,
    ) -> R {
        let mut adapters = self.target_adapters.lock().await;
        f(&mut adapters)
    }

    pub async fn with_provider_statuses<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, ProviderStatus>) -> R,
    ) -> R {
        let mut statuses = self.statuses.lock().await;
        f(&mut statuses)
    }
}
