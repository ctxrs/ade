use std::sync::Arc;

use crate::api::providers::login::resolve_runtime_provider_command_from_config;
use crate::daemon::AppState;
use ctx_managed_installs as installer;

mod process;
mod shim;

pub(super) use process::{spawn_claude_setup_token_command, ClaudeLoginSpawn};
#[cfg(test)]
pub(super) use shim::{
    claude_browser_open_shim_script, claude_login_should_skip_browser_open,
    CLAUDE_BROWSER_AUTH_TIER,
};

pub(super) async fn resolve_claude_login_runtime_from_config(
    data_root: &std::path::Path,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    if let Some(runtime_command) =
        resolve_runtime_provider_command_from_config(data_root, "claude-cli").await?
    {
        return Ok(runtime_command);
    }

    anyhow::bail!(
        "runtime_command_missing: provider=claude-cli (ctx requires a managed or explicitly configured Claude CLI runtime command; host PATH lookup is not supported)"
    )
}

pub(super) async fn resolve_claude_login_runtime(
    state: &Arc<AppState>,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_claude_login_runtime_from_config(&state.core.data_root).await
}
