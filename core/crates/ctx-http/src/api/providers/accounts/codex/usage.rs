use super::*;

use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_provider_runtime::provider_usage;

pub(crate) async fn get_codex_accounts_usage(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<CodexAccountsUsageResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let refresh = query.refresh.unwrap_or(false);
    let registry = provider_accounts::load_codex_registry(&state.core.data_root)
        .await
        .map_err(internal_error)?;
    let active_id = registry.active_account_id.clone();
    let cached_active = if !refresh {
        state
            .providers
            .provider_usage_cache_entry(CODEX_PROVIDER_ID)
            .await
    } else {
        None
    };

    let to_err = |e: anyhow::Error| internal_error(e);

    let mut entries = Vec::new();
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        return Err(internal_error(config_error));
    }

    for account in registry.accounts {
        let _ = provider_accounts::hydrate_codex_account_home_from_secret(
            &state.core.data_root,
            &account.id,
        )
        .await;
        let mut env = provider_accounts::codex_env_for_account(&state.core.data_root, &account.id);
        crate::daemon::installer::ensure_codex_cli_command_env_for_target(
            &mut env,
            &cfg,
            CODEX_PROVIDER_ID,
            Some(ctx_provider_install::install_state::InstallTarget::Host),
        )
        .map_err(to_err)?;
        let usage = if active_id.as_deref() == Some(&account.id) {
            if let Some(snapshot) = cached_active.clone() {
                snapshot
            } else {
                provider_usage::fetch_codex_usage_snapshot(env)
                    .await
                    .map_err(to_err)?
            }
        } else {
            provider_usage::fetch_codex_usage_snapshot(env)
                .await
                .map_err(to_err)?
        };
        entries.push(CodexAccountUsageEntry {
            account_id: Some(account.id),
            label: account.label,
            email: account.email,
            plan_type: account.plan_type,
            last_used_at: account.last_used_at,
            usage,
        });
    }

    Ok(Json(CodexAccountsUsageResponse { entries }))
}
