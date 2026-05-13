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
    store_provider_verify_cache_value(state, ws_id, install_target, provider_id, verify_value)
        .await;
}
