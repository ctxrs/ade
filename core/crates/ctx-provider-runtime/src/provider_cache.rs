use std::collections::HashMap;

use ctx_core::ids::WorkspaceId;
use ctx_provider_install::install_state::InstallTarget;

use crate::{provider_usage, CachedProviderOptions, CachedProviderVerify, ProviderRuntime};

impl ProviderRuntime {
    pub async fn with_provider_options_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderOptions>) -> R,
    ) -> R {
        let mut cache = self.options_cache.lock().await;
        f(&mut cache)
    }

    pub async fn with_provider_verify_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderVerify>) -> R,
    ) -> R {
        let mut cache = self.verify_cache.lock().await;
        f(&mut cache)
    }

    pub async fn with_provider_usage_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_usage::ProviderUsageSnapshot>) -> R,
    ) -> R {
        let mut cache = self.usage_cache.lock().await;
        f(&mut cache)
    }
}

pub fn workspace_provider_cache_key(
    workspace_id: WorkspaceId,
    target: InstallTarget,
    provider_id: &str,
) -> String {
    format!("{}/{}/{}", workspace_id.0, target.as_str(), provider_id)
}

pub fn cache_key_matches_provider(cache_key: &str, provider_id: &str) -> bool {
    cache_key
        .rsplit_once('/')
        .is_some_and(|(_, key_provider)| key_provider == provider_id)
}

pub async fn invalidate_provider_probe_caches(runtime: &ProviderRuntime, provider_id: &str) {
    runtime
        .options_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
    runtime
        .verify_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
}

pub async fn invalidate_workspace_provider_options_cache(
    runtime: &ProviderRuntime,
    workspace_id: WorkspaceId,
    provider_id: &str,
) {
    let key_prefix = format!("{}/", workspace_id.0);
    let key_suffix = format!("/{provider_id}");
    runtime
        .options_cache
        .lock()
        .await
        .retain(|key, _| !(key.starts_with(&key_prefix) && key.ends_with(&key_suffix)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_provider_match_only_checks_provider_segment() {
        assert!(cache_key_matches_provider("workspace/host/codex", "codex"));
        assert!(!cache_key_matches_provider(
            "workspace/host/codex-crp",
            "codex"
        ));
        assert!(!cache_key_matches_provider(
            "workspace/host/codex/extra",
            "codex"
        ));
        assert!(!cache_key_matches_provider("codex", "codex"));
        assert!(!cache_key_matches_provider("not-a-key", "codex"));
    }
}
