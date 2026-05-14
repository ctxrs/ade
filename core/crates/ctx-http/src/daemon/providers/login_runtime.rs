use std::{path::Path, sync::Arc};

use anyhow::Context;

use crate::daemon::DaemonState;

#[derive(Debug, Clone)]
pub(crate) struct ProviderLoginRuntimeCommand {
    pub(crate) command_abs_path: String,
    pub(crate) args: Vec<String>,
}

impl From<ctx_managed_installs::ProviderRuntimeCommand> for ProviderLoginRuntimeCommand {
    fn from(value: ctx_managed_installs::ProviderRuntimeCommand) -> Self {
        Self {
            command_abs_path: value.command_abs_path,
            args: value.args,
        }
    }
}

async fn resolve_runtime_provider_command_from_config(
    data_root: &Path,
    provider_id: &str,
) -> anyhow::Result<Option<ctx_managed_installs::ProviderRuntimeCommand>> {
    let cfg = ctx_managed_installs::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    ctx_managed_installs::resolve_runtime_provider_command(&cfg, provider_id)
        .with_context(|| format!("resolving runtime command for {provider_id}"))
}

async fn resolve_provider_login_command_from_config(
    data_root: &Path,
    provider_id: &str,
) -> anyhow::Result<Option<ctx_managed_installs::ProviderRuntimeCommand>> {
    let cfg = ctx_managed_installs::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    ctx_managed_installs::resolve_provider_login_command(&cfg, provider_id)
        .with_context(|| format!("resolving prepared login executable for {provider_id}"))
}

fn is_cursor_login_command(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "cursor-agent" || name == "cursor-agent.exe")
}

pub(crate) async fn resolve_cursor_login_runtime_from_config(
    data_root: &Path,
) -> anyhow::Result<ProviderLoginRuntimeCommand> {
    if let Some(runtime) = resolve_provider_login_command_from_config(data_root, "cursor").await? {
        if is_cursor_login_command(Path::new(&runtime.command_abs_path)) {
            return Ok(runtime.into());
        }
        anyhow::bail!(
            "runtime_command_invalid: provider=cursor-login (configured login executable must point to `cursor-agent`)"
        );
    }

    if let Some(runtime) = resolve_runtime_provider_command_from_config(data_root, "cursor").await?
    {
        if matches!(
            runtime.source,
            ctx_managed_installs::ProviderRuntimeCommandSource::BundledSeed
        ) {
            anyhow::bail!(
                "runtime_command_missing: provider=cursor-login (ctx requires a managed or explicitly configured `cursor-agent` login executable; bundled runtime discovery is not supported)"
            );
        }
        if is_cursor_login_command(Path::new(&runtime.command_abs_path)) {
            return Ok(runtime.into());
        }
        anyhow::bail!(
            "runtime_command_invalid: provider=cursor-login (configured runtime command must point to `cursor-agent`)"
        );
    }

    anyhow::bail!(
        "runtime_command_missing: provider=cursor-login (ctx requires a managed or explicitly configured `cursor-agent` login executable; host PATH lookup is not supported)"
    );
}

pub(crate) async fn resolve_cursor_login_runtime(
    state: &Arc<DaemonState>,
) -> anyhow::Result<ProviderLoginRuntimeCommand> {
    resolve_cursor_login_runtime_from_config(&state.core.data_root).await
}

pub(crate) async fn resolve_claude_login_runtime_from_config(
    data_root: &Path,
) -> anyhow::Result<ProviderLoginRuntimeCommand> {
    if let Some(runtime_command) =
        resolve_runtime_provider_command_from_config(data_root, "claude-cli").await?
    {
        return Ok(runtime_command.into());
    }

    anyhow::bail!(
        "runtime_command_missing: provider=claude-cli (ctx requires a managed or explicitly configured Claude CLI runtime command; host PATH lookup is not supported)"
    )
}

pub(crate) async fn resolve_claude_login_runtime(
    state: &Arc<DaemonState>,
) -> anyhow::Result<ProviderLoginRuntimeCommand> {
    resolve_claude_login_runtime_from_config(&state.core.data_root).await
}
