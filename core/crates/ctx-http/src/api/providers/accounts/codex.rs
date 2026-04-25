use super::common::{bad_request, internal_error, provider_account_delete_error};
use super::*;

pub(crate) async fn codex_accounts_response(state: &Arc<AppState>) -> CodexAccountsResponse {
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    let logins = {
        let map = state.providers.codex_login_sessions.lock().await;
        map.values().cloned().collect::<Vec<_>>()
    };
    CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }
}

pub(crate) async fn list_codex_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(codex_accounts_response(&state).await))
}

pub(crate) async fn probe_host_codex_import(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<provider_accounts::CodexHostImportProbe>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        provider_accounts::probe_host_codex_auth_candidate().await,
    ))
}

pub(crate) async fn import_host_codex_auth(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexHostImportReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::import_host_codex_auth_to_secret_store(&state.core.data_root, req.label)
        .await
        .map_err(bad_request)?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    Ok(Json(codex_accounts_response(&state).await))
}

pub(crate) async fn get_codex_accounts_usage(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<CodexAccountsUsageResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let refresh = query.refresh.unwrap_or(false);
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    let active_id = registry.active_account_id.clone();
    let cached_active = if !refresh {
        let cache = state.providers.usage_cache.lock().await;
        cache.get("codex-crp").cloned()
    } else {
        None
    };

    let to_err = |e: anyhow::Error| internal_error(e);

    let mut entries = Vec::new();
    let (cfg, config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
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
        crate::installer::ensure_codex_cli_command_env_for_target(
            &mut env,
            &cfg,
            "codex-crp",
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

pub(crate) async fn set_codex_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexActiveAccountReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    let registry =
        provider_accounts::set_active_codex_account(&state.core.data_root, req.account_id)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                let status = if msg.contains("api_shape=openai_responses")
                    || msg.contains("auth_type=bearer")
                    || msg.contains("unknown account")
                {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (status, Json(ApiErrorResp { error: msg }))
            })?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    let logins = {
        let map = state.providers.codex_login_sessions.lock().await;
        map.values().cloned().collect::<Vec<_>>()
    };
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}

pub(crate) async fn delete_codex_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let registry = provider_accounts::remove_codex_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    let logins = {
        let mut map = state.providers.codex_login_sessions.lock().await;
        map.remove(&id);
        map.values().cloned().collect::<Vec<_>>()
    };
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}
