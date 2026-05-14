use std::sync::Arc;
use std::time::Duration;

use ctx_core::ids::WorkspaceId;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::provider_cache::{
    workspace_provider_cache_key, CachedProviderJsonSnapshot,
};
use ctx_provider_runtime::provider_launch::options::provider_options_cache_entry_is_authoritative;

use crate::daemon::DaemonState;

pub(crate) struct ProviderOptionsCacheSnapshot {
    cache_key: String,
    verify_entry: Option<CachedProviderJsonSnapshot>,
    authoritative_entry: Option<CachedProviderJsonSnapshot>,
}

impl ProviderOptionsCacheSnapshot {
    pub(crate) async fn load(
        state: &Arc<DaemonState>,
        workspace_id: WorkspaceId,
        target: InstallTarget,
        provider_id: &str,
        skip_cached_config_surfaces: bool,
    ) -> Self {
        let cache_key = workspace_provider_cache_key(workspace_id, target, provider_id);
        let verify_entry = if skip_cached_config_surfaces {
            None
        } else {
            state
                .providers
                .provider_verify_cache_entry(&cache_key)
                .await
        };
        let cached_entry = if skip_cached_config_surfaces {
            None
        } else {
            state
                .providers
                .provider_options_cache_entry(&cache_key)
                .await
        };
        let authoritative_entry = cached_entry
            .filter(|entry| entry.value.get("config_error").is_none())
            .filter(|entry| {
                provider_options_cache_entry_is_authoritative(provider_id, &entry.value)
            });

        Self {
            cache_key,
            verify_entry,
            authoritative_entry,
        }
    }

    pub(crate) fn fresh_authoritative_response(
        &self,
        cache_ttl: Duration,
        verify_ttl: Duration,
    ) -> Option<serde_json::Value> {
        let entry = self.authoritative_entry.as_ref()?;
        if entry.cached_at.elapsed() >= cache_ttl {
            return None;
        }
        let mut out = entry.value.clone();
        self.attach_verify_cache(&mut out, verify_ttl);
        Some(out)
    }

    fn cached_payload_field(&self, field: &str) -> Option<serde_json::Value> {
        self.authoritative_entry
            .as_ref()
            .and_then(|entry| entry.value.get(field))
            .cloned()
            .filter(|value| !value.is_null())
    }

    pub(crate) fn cached_models(&self) -> Option<serde_json::Value> {
        self.cached_payload_field("models")
    }

    pub(crate) fn cached_modes(&self) -> Option<serde_json::Value> {
        self.cached_payload_field("modes")
    }

    pub(crate) async fn store_response(&self, state: &Arc<DaemonState>, value: serde_json::Value) {
        state
            .providers
            .store_provider_options_cache_value(self.cache_key.clone(), value)
            .await;
    }

    pub(crate) fn attach_verify_cache(&self, value: &mut serde_json::Value, verify_ttl: Duration) {
        attach_verify_cache(value, self.verify_entry.as_ref(), verify_ttl);
    }
}

pub(crate) async fn store_provider_verify_cache_value(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    target: InstallTarget,
    provider_id: &str,
    value: serde_json::Value,
) {
    let cache_key = workspace_provider_cache_key(workspace_id, target, provider_id);
    state
        .providers
        .store_provider_verify_cache_value(cache_key, value)
        .await;
}

fn attach_verify_cache(
    value: &mut serde_json::Value,
    verify_entry: Option<&CachedProviderJsonSnapshot>,
    verify_ttl: Duration,
) {
    if let Some(verify_entry) = verify_entry {
        if verify_entry.cached_at.elapsed() < verify_ttl {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("verify".to_string(), verify_entry.value.clone());
            }
        }
    }
}
