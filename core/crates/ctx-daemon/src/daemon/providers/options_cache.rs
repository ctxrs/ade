use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_provider_install::install_state::InstallTarget;

pub use ctx_provider_runtime::provider_options::cache::ProviderOptionsCacheSnapshot;

use crate::daemon::DaemonState;

pub async fn store_provider_verify_cache_value(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    target: InstallTarget,
    provider_id: &str,
    value: serde_json::Value,
) {
    ctx_provider_runtime::provider_options::cache::store_provider_verify_cache_value(
        &state.providers,
        workspace_id,
        target,
        provider_id,
        value,
    )
    .await;
}
