use ctx_provider_runtime::ProviderRuntime;

use super::ProviderCacheStats;

pub(super) async fn collect_provider_cache_stats(
    providers: &ProviderRuntime,
) -> ProviderCacheStats {
    let stats = providers.cache_stats().await;

    ProviderCacheStats {
        adapters: stats.adapters,
        statuses: stats.statuses,
        options_cache: stats.options_cache,
        verify_cache: stats.verify_cache,
        usage_cache: stats.usage_cache,
        installs: stats.installs,
    }
}
