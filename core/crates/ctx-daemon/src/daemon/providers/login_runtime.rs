use std::sync::Arc;

use ctx_provider_runtime::provider_login_runtime::{
    resolve_claude_login_runtime_from_config, resolve_cursor_login_runtime_from_config,
    ProviderLoginRuntimeCommand,
};

use crate::daemon::DaemonState;

pub async fn resolve_cursor_login_runtime(
    state: &Arc<DaemonState>,
) -> anyhow::Result<ProviderLoginRuntimeCommand> {
    resolve_cursor_login_runtime_from_config(&state.core.data_root).await
}

pub async fn resolve_claude_login_runtime(
    state: &Arc<DaemonState>,
) -> anyhow::Result<ProviderLoginRuntimeCommand> {
    resolve_claude_login_runtime_from_config(&state.core.data_root).await
}
