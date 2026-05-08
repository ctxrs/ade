use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use ctx_core::models::Session;

use crate::daemon::AppState;

use super::turn_start::turn_start_deadline;

mod mcp;
mod openhands;
#[cfg(test)]
mod tests;

pub(super) struct PreparedProviderLaunchEnvironment {
    pub(super) mcp_token: Option<String>,
    pub(super) codex_home: Option<PathBuf>,
    pub(super) start_deadline_duration: Duration,
}

pub(super) async fn prepare_provider_launch_environment(
    state: &Arc<AppState>,
    session: &Session,
    runtime_provider_id: &str,
    workdir: &Path,
    provider_env: &mut HashMap<String, String>,
) -> Result<PreparedProviderLaunchEnvironment> {
    apply_provider_launch_overrides(runtime_provider_id, workdir, provider_env).await?;
    let mcp_disabled = provider_env
        .get("CTX_MCP_DISABLED")
        .and_then(|value| ctx_core::boolish::parse_boolish(value))
        .unwrap_or(false);
    let mcp_token =
        mcp::issue_mcp_token_if_enabled(state, session, provider_env, mcp_disabled).await;
    let codex_home = provider_env
        .get("CODEX_HOME")
        .map(|value| PathBuf::from(value.as_str()));
    let start_deadline_duration = turn_start_deadline(provider_env);

    Ok(PreparedProviderLaunchEnvironment {
        mcp_token,
        codex_home,
        start_deadline_duration,
    })
}

pub(super) async fn apply_provider_launch_overrides(
    provider_id: &str,
    workdir: &Path,
    provider_env: &mut HashMap<String, String>,
) -> Result<()> {
    mcp::apply_provider_mcp_command_overrides(provider_id, provider_env);

    if provider_id == "openhands" {
        openhands::apply_openhands_launch_overrides(workdir, provider_env).await?;
        return Ok(());
    }

    mcp::strip_unused_daemon_auth_from_provider_env(provider_env);
    Ok(())
}
