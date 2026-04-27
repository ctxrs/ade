#[cfg(test)]
pub(super) use super::super::provider_probe_auth::endpoint_selection_is_active;
pub(super) use super::super::provider_probe_auth::provider_auth_mode;
#[cfg(test)]
pub(super) use super::super::provider_probe_auth::provider_has_active_auth_config;
use super::*;

pub(super) fn cache_key_matches_provider(cache_key: &str, provider_id: &str) -> bool {
    cache_key
        .rsplit('/')
        .next()
        .is_some_and(|key_provider| key_provider == provider_id)
}

pub(super) async fn invalidate_provider_probe_caches(state: &Arc<AppState>, provider_id: &str) {
    state
        .providers
        .options_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
    state
        .providers
        .verify_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
}

pub(super) fn bootstrap_provider_probe_summary(
    provider_status: &ProviderStatus,
    has_active_auth: bool,
) -> (bool, bool, Option<String>) {
    if !crate::provider_usability::provider_status_is_usable(provider_status) {
        return (
            false,
            false,
            Some(
                crate::provider_usability::provider_status_unusable_reason(provider_status)
                    .unwrap_or_else(|| "provider not ready for use".to_string()),
            ),
        );
    }

    (true, !has_active_auth, None)
}
