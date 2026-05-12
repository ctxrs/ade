use super::*;

pub(super) struct CachedProviderOptionsSnapshot {
    pub(super) cached_at: std::time::Instant,
    pub(super) value: serde_json::Value,
}

pub(in crate::api::provider_launch) struct ProviderOptionsCacheSnapshot {
    cache_key: String,
    verify_entry: Option<CachedProviderOptionsSnapshot>,
    authoritative_entry: Option<CachedProviderOptionsSnapshot>,
}

impl ProviderOptionsCacheSnapshot {
    pub(in crate::api::provider_launch) async fn load(
        state: &Arc<AppState>,
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
                .with_provider_verify_cache(|cache| {
                    cache
                        .get(&cache_key)
                        .map(|c| CachedProviderOptionsSnapshot {
                            cached_at: c.cached_at,
                            value: c.value.clone(),
                        })
                })
                .await
        };
        let cached_entry = if skip_cached_config_surfaces {
            None
        } else {
            state
                .providers
                .with_provider_options_cache(|cache| {
                    cache
                        .get(&cache_key)
                        .map(|c| CachedProviderOptionsSnapshot {
                            cached_at: c.cached_at,
                            value: c.value.clone(),
                        })
                })
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

    pub(in crate::api::provider_launch) fn fresh_authoritative_response(
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

    pub(in crate::api::provider_launch) fn cached_models(&self) -> Option<serde_json::Value> {
        self.cached_payload_field("models")
    }

    pub(in crate::api::provider_launch) fn cached_modes(&self) -> Option<serde_json::Value> {
        self.cached_payload_field("modes")
    }

    pub(in crate::api::provider_launch) async fn store_response(
        &self,
        state: &Arc<AppState>,
        value: serde_json::Value,
    ) {
        state
            .providers
            .with_provider_options_cache(|cache| {
                cache.insert(
                    self.cache_key.clone(),
                    crate::daemon::CachedProviderOptions {
                        cached_at: std::time::Instant::now(),
                        value,
                    },
                );
            })
            .await;
    }

    pub(in crate::api::provider_launch) fn attach_verify_cache(
        &self,
        value: &mut serde_json::Value,
        verify_ttl: Duration,
    ) {
        attach_verify_cache(value, self.verify_entry.as_ref(), verify_ttl);
    }
}

fn attach_verify_cache(
    value: &mut serde_json::Value,
    verify_entry: Option<&CachedProviderOptionsSnapshot>,
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
