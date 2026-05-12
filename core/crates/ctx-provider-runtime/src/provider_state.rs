use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ctx_provider_matrix::{MatrixRefreshOutcome, ProviderMatrix};
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

    pub async fn provider_adapter(&self, provider_id: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters.lock().await.get(provider_id).cloned()
    }

    pub async fn has_provider_adapter(&self, provider_id: &str) -> bool {
        self.adapters.lock().await.contains_key(provider_id)
    }

    pub async fn provider_adapter_count(&self) -> usize {
        self.adapters.lock().await.len()
    }

    pub async fn provider_adapter_entries(&self) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
        self.adapters
            .lock()
            .await
            .iter()
            .map(|(id, adapter)| (id.clone(), Arc::clone(adapter)))
            .collect()
    }

    pub async fn upsert_target_provider_adapter(
        &self,
        cache_key: String,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.target_adapters.lock().await.insert(cache_key, adapter);
    }

    pub async fn target_provider_adapter(
        &self,
        cache_key: &str,
    ) -> Option<Arc<dyn ProviderAdapter>> {
        self.target_adapters.lock().await.get(cache_key).cloned()
    }

    pub async fn has_target_provider_adapter(&self, cache_key: &str) -> bool {
        self.target_adapters.lock().await.contains_key(cache_key)
    }

    pub async fn target_provider_adapter_count(&self) -> usize {
        self.target_adapters.lock().await.len()
    }

    pub async fn target_provider_adapter_entries(&self) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
        self.target_adapters
            .lock()
            .await
            .iter()
            .map(|(id, adapter)| (id.clone(), Arc::clone(adapter)))
            .collect()
    }

    pub async fn all_provider_adapter_entries(&self) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
        let mut adapters = self.provider_adapter_entries().await;
        adapters.extend(self.target_provider_adapter_entries().await);
        adapters
    }

    pub async fn provider_adapter_entries_for_provider(
        &self,
        provider_id: &str,
    ) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
        let mut adapters = Vec::new();
        if let Some(adapter) = self.provider_adapter(provider_id).await {
            adapters.push((provider_id.to_string(), adapter));
        }
        let target_prefix = format!("{provider_id}@");
        adapters.extend(
            self.target_provider_adapter_entries()
                .await
                .into_iter()
                .filter(|(id, _)| id.starts_with(&target_prefix)),
        );
        adapters
    }

    pub async fn replace_provider_statuses(&self, statuses: HashMap<String, ProviderStatus>) {
        *self.statuses.lock().await = statuses;
    }

    pub async fn upsert_provider_status(&self, provider_id: String, status: ProviderStatus) {
        self.statuses.lock().await.insert(provider_id, status);
    }

    pub async fn provider_status(&self, provider_id: &str) -> Option<ProviderStatus> {
        self.statuses.lock().await.get(provider_id).cloned()
    }

    pub async fn has_provider_status(&self, provider_id: &str) -> bool {
        self.statuses.lock().await.contains_key(provider_id)
    }

    pub async fn provider_status_count(&self) -> usize {
        self.statuses.lock().await.len()
    }

    pub async fn provider_status_ids(&self) -> Vec<String> {
        self.statuses.lock().await.keys().cloned().collect()
    }

    pub async fn provider_statuses(&self) -> Vec<ProviderStatus> {
        self.statuses.lock().await.values().cloned().collect()
    }

    pub async fn load_provider_matrix(&self, data_root: &Path) -> ProviderMatrix {
        ctx_provider_matrix::load_matrix_cached(data_root, &self.matrix_cache).await
    }

    pub async fn invalidate_provider_matrix_cache(&self) {
        ctx_provider_matrix::invalidate_matrix_cache(&self.matrix_cache).await;
    }

    pub async fn refresh_provider_matrix_from_local_sources(
        &self,
        data_root: &Path,
    ) -> MatrixRefreshOutcome {
        self.invalidate_provider_matrix_cache().await;
        ctx_provider_matrix::refresh_matrix_from_local_sources(data_root, &self.matrix_cache).await
    }
}
