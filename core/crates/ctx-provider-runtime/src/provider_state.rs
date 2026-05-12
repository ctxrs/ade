use std::collections::HashMap;
use std::sync::Arc;

use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};

use crate::ProviderRuntime;

impl ProviderRuntime {
    pub async fn inspect_provider_adapters(&self) -> Vec<(String, Result<ProviderStatus, String>)> {
        let adapters = self.adapters.lock().await;
        let mut statuses = Vec::with_capacity(adapters.len());
        for (id, adapter) in adapters.iter() {
            statuses.push((
                id.clone(),
                adapter.inspect().await.map_err(|err| err.to_string()),
            ));
        }
        statuses
    }

    pub async fn upsert_provider_adapter(
        &self,
        provider_id: String,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.adapters.lock().await.insert(provider_id, adapter);
    }

    pub async fn upsert_target_provider_adapter(
        &self,
        cache_key: String,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.target_adapters.lock().await.insert(cache_key, adapter);
    }

    pub async fn replace_provider_statuses(&self, statuses: HashMap<String, ProviderStatus>) {
        *self.statuses.lock().await = statuses;
    }

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
