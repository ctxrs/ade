use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_core::redaction::redact_json_value;
use ctx_provider_install::install_state::InstallTarget;

use crate::daemon::providers::{store_provider_verify_cache_value, ProviderAuthCheckSnapshot};
use crate::daemon::DaemonState;

pub(super) async fn store_provider_auth_check_cache(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    install_target: InstallTarget,
    provider_id: &str,
    snapshot: &ProviderAuthCheckSnapshot,
) {
    let verify_value =
        redact_json_value(serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null));
    store_provider_verify_cache_value(
        state,
        workspace_id,
        install_target,
        provider_id,
        verify_value,
    )
    .await;
}
