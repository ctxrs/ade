use super::*;
use ctx_provider_runtime::provider_usability::{
    provider_status_is_usable, provider_status_unusable_reason,
};

pub(super) async fn invalidate_provider_probe_caches(state: &Arc<AppState>, provider_id: &str) {
    ctx_provider_runtime::provider_cache::invalidate_provider_probe_caches(
        &state.providers,
        provider_id,
    )
    .await;
}

pub(super) fn bootstrap_provider_probe_summary(
    provider_status: &ProviderStatus,
    has_active_auth: bool,
) -> (bool, bool, Option<String>) {
    if !provider_status_is_usable(provider_status) {
        return (
            false,
            false,
            Some(
                provider_status_unusable_reason(provider_status)
                    .unwrap_or_else(|| "provider not ready for use".to_string()),
            ),
        );
    }

    (true, !has_active_auth, None)
}
