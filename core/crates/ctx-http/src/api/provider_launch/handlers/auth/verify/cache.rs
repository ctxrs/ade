use ctx_core::ids::WorkspaceId;

use super::*;

pub(super) async fn store_provider_verify_cache(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
    install_target: InstallTarget,
    provider_id: &str,
    resp: &ProviderAuthCheckResp,
) {
    let verify_value =
        redact_json_value(serde_json::to_value(resp).unwrap_or(serde_json::Value::Null));
    let cache_key = workspace_provider_cache_key(ws_id, install_target, provider_id);
    state
        .providers
        .with_provider_verify_cache(|cache| {
            cache.insert(
                cache_key,
                crate::daemon::CachedProviderVerify {
                    cached_at: std::time::Instant::now(),
                    value: verify_value,
                },
            );
        })
        .await;
}
