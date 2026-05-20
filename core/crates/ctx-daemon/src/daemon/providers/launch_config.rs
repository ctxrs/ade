use std::sync::Arc;

pub use ctx_provider_runtime::provider_launch::config_snapshot::{
    ProviderLaunchConfigError, ProviderLaunchConfigSnapshot,
};

use crate::daemon::DaemonState;

pub async fn load_provider_launch_config_snapshot(
    state: &Arc<DaemonState>,
    provider_id: &str,
) -> ProviderLaunchConfigSnapshot {
    ctx_provider_runtime::provider_launch::config_snapshot::load_provider_launch_config_snapshot(
        state.as_ref(),
        provider_id,
    )
    .await
}
