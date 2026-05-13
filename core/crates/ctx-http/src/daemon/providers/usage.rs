use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
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

pub(crate) struct CodexAccountUsageRecord {
    pub(crate) account_id: Option<String>,
    pub(crate) label: String,
    pub(crate) email: Option<String>,
    pub(crate) plan_type: Option<String>,
    pub(crate) last_used_at: Option<DateTime<Utc>>,
    pub(crate) usage: provider_usage::ProviderUsageSnapshot,
}

pub(crate) async fn load_codex_accounts_usage(
    state: &Arc<AppState>,
    refresh: bool,
) -> Result<Vec<CodexAccountUsageRecord>> {
    let registry = ctx_provider_accounts::load_codex_registry(&state.core.data_root).await?;
    let active_id = registry.active_account_id.clone();
    let cached_active = if !refresh {
        state
            .providers
            .provider_usage_cache_entry(CODEX_PROVIDER_ID)
            .await
    } else {
        None
    };
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        anyhow::bail!(config_error);
    }

    let mut entries = Vec::new();
    for account in registry.accounts {
        let _ = ctx_provider_accounts::hydrate_codex_account_home_from_secret(
            &state.core.data_root,
            &account.id,
        )
        .await;
        let mut env =
            ctx_provider_accounts::codex_env_for_account(&state.core.data_root, &account.id);
        ctx_managed_installs::ensure_codex_cli_command_env_for_target(
            &mut env,
            &cfg,
            CODEX_PROVIDER_ID,
            Some(ctx_provider_install::InstallTarget::Host),
        )?;
        let usage = if active_id.as_deref() == Some(&account.id) {
            if let Some(snapshot) = cached_active.clone() {
                snapshot
            } else {
                provider_usage::fetch_codex_usage_snapshot(env).await?
            }
        } else {
            provider_usage::fetch_codex_usage_snapshot(env).await?
        };
        entries.push(CodexAccountUsageRecord {
            account_id: Some(account.id),
            label: account.label,
            email: account.email,
            plan_type: account.plan_type,
            last_used_at: account.last_used_at,
            usage,
        });
    }

    Ok(entries)
}
