use crate::daemon::AppState;

use super::ProviderCacheStats;

pub(super) async fn collect_provider_cache_stats(state: &AppState) -> ProviderCacheStats {
    let adapters = state
        .providers
        .with_provider_adapters(|adapters| adapters.len())
        .await;
    let statuses = state
        .providers
        .with_provider_statuses(|statuses| statuses.len())
        .await;
    let options_cache = state
        .providers
        .with_provider_options_cache(|cache| cache.len())
        .await;
    let verify_cache = state
        .providers
        .with_provider_verify_cache(|cache| cache.len())
        .await;
    let usage_cache = state
        .providers
        .with_provider_usage_cache(|cache| cache.len())
        .await;
    let installs = state.providers.install_count().await;

    ProviderCacheStats {
        adapters,
        statuses,
        options_cache,
        verify_cache,
        usage_cache,
        installs,
    }
}
