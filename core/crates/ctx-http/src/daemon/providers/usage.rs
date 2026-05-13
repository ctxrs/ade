use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_provider_runtime::provider_usage;

use crate::daemon::AppState;

async fn provider_usage_env(
    state: &Arc<AppState>,
    provider_id: &str,
) -> Result<HashMap<String, String>> {
    if provider_id != CODEX_PROVIDER_ID {
        return Ok(HashMap::new());
    }

    let mut env =
        ctx_provider_accounts::codex_env_for_active_account(&state.core.data_root).await?;
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        anyhow::bail!(config_error);
    }
    ctx_managed_installs::ensure_codex_cli_command_env_for_target(
        &mut env,
        &cfg,
        CODEX_PROVIDER_ID,
        Some(ctx_provider_install::InstallTarget::Host),
    )?;
    Ok(env)
}

pub(crate) async fn load_provider_usage(
    state: &Arc<AppState>,
    provider_id: &str,
    refresh: bool,
) -> Result<provider_usage::ProviderUsageSnapshot> {
    let env = provider_usage_env(state, provider_id).await?;
    if !refresh {
        if let Some(snapshot) = state
            .providers
            .provider_usage_cache_entry(provider_id)
            .await
        {
            return Ok(snapshot);
        }
    }
    provider_usage::refresh_provider_usage_for(state.as_ref(), provider_id, env).await
}
