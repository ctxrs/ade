use super::catalog::build_model_catalog;
use super::*;

async fn load_pinned_subscription_model_catalog(
    state: &Arc<AppState>,
    provider_id: &str,
    install_target: ctx_provider_install::install_state::InstallTarget,
) -> Result<Option<ModelCatalog>, String> {
    let (managed, config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        return Err(config_error);
    }
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let provider_status = crate::api::providers::provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        provider_id,
        install_target,
    )
    .await;
    let models_value = provider_accounts::pinned_subscription_models_value(
        provider_id,
        provider_status.version.as_deref(),
    );
    Ok(models_value.and_then(|value| build_model_catalog(&value)))
}

async fn load_provider_model_catalog_for_install_target(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    install_target: ctx_provider_install::install_state::InstallTarget,
) -> Result<Option<ModelCatalog>, String> {
    let provider_id = canonical_provider_id(provider_id);
    let cache_key = format!(
        "{}/{}/{}",
        workspace.id.0,
        install_target.as_str(),
        provider_id
    );
    if let Some(entry) = state
        .providers
        .options_cache
        .lock()
        .await
        .get(&cache_key)
        .filter(|entry| {
            crate::api::provider_catalog::provider_options_cache_entry_is_authoritative(
                provider_id,
                &entry.value,
            )
        })
    {
        if let Some(models) = entry.value.get("models") {
            if let Some(catalog) = build_model_catalog(models) {
                return Ok(Some(catalog));
            }
        }
    }

    let (source_config, source_config_error) =
        crate::api::provider_launch::load_provider_source_config_with_error(
            &state.core.data_root,
            provider_id,
        )
        .await;
    if let Some(config_error) = source_config_error {
        return Err(config_error);
    }
    if let Some(config) = source_config.as_ref() {
        if config.selected_source_kind == harness_sources::HarnessSourceKind::Endpoint {
            let selected_endpoint_id = config.selected_endpoint_id.as_deref().ok_or_else(|| {
                format!(
                    "selected source is endpoint for '{provider_id}' but no endpoint is selected"
                )
            })?;
            let endpoint = config
                .endpoints
                .iter()
                .find(|candidate| candidate.id == selected_endpoint_id)
                .ok_or_else(|| {
                    format!(
                        "selected endpoint '{selected_endpoint_id}' for '{provider_id}' was not found"
                    )
                })?;

            let now = chrono::Utc::now();
            if harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
                let data_root = state.core.data_root.clone();
                let provider_id_for_refresh = provider_id.to_string();
                let endpoint_id_for_refresh = endpoint.id.clone();
                tokio::spawn(async move {
                    let _ = harness_sources::refresh_provider_endpoint_model_catalog(
                        &data_root,
                        &provider_id_for_refresh,
                        &endpoint_id_for_refresh,
                    )
                    .await;
                });
            }

            let models_value = serde_json::json!({
                "models": endpoint.model_catalog_models,
                "current_model_id": endpoint.model_override,
            });
            if let Some(models) = build_model_catalog(&models_value) {
                let mut value = serde_json::json!({
                    "provider_id": provider_id,
                    "workspace_id": workspace.id.0,
                    "installed": true,
                    "probe_ok": true,
                    "supports_load": false,
                    "auth_required": false,
                    "models": models_value,
                    "probed_at": now.to_rfc3339(),
                });
                value["source"] = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
                value = redact_json_value(value);
                state.providers.options_cache.lock().await.insert(
                    cache_key,
                    crate::daemon::CachedProviderOptions {
                        cached_at: std::time::Instant::now(),
                        value,
                    },
                );
                return Ok(Some(models));
            }

            return Ok(None);
        }
    }

    if !crate::api::provider_catalog::provider_supports_runtime_model_catalog(provider_id) {
        return load_pinned_subscription_model_catalog(state, provider_id, install_target).await;
    }

    let pinned_catalog =
        load_pinned_subscription_model_catalog(state, provider_id, install_target).await?;

    let (cfg, config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        return Err(config_error);
    }
    let runtime_command = match installer::resolve_runtime_provider_command_for_target(
        &cfg,
        provider_id,
        Some(install_target),
    ) {
        Ok(Some(command)) => command,
        Ok(None) => return Ok(pinned_catalog),
        Err(err) => {
            tracing::warn!(
                provider_id = provider_id,
                "provider runtime command resolution failed: {}",
                logs::redact_sensitive(&err.to_string())
            );
            return Ok(pinned_catalog);
        }
    };
    let command = runtime_command.command_abs_path;
    let args = runtime_command.args;

    let probe_context =
        match crate::provider_launch::probe::provider_probe_context_for_workspace_runtime(
            state.as_ref(),
            workspace,
            provider_id,
        )
        .await
        {
            Ok(context) => context,
            Err(err) => {
                tracing::warn!(
                    provider_id = provider_id,
                    "provider probe runtime context failed: {}",
                    logs::redact_sensitive(&err)
                );
                return Ok(pinned_catalog);
            }
        };
    let mut env = probe_context.env;
    installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut env,
        &cfg,
        provider_id,
        &state.core.data_root,
        Some(install_target),
    );
    if let Err(err) = installer::ensure_codex_cli_command_env_for_target(
        &mut env,
        &cfg,
        provider_id,
        Some(install_target),
    ) {
        tracing::warn!(
            provider_id = provider_id,
            "provider codex-cli runtime path resolution failed: {}",
            logs::redact_sensitive(&err.to_string())
        );
        return Ok(pinned_catalog);
    }

    let probe = match probe_crp_models(provider_id, command, args, probe_context.cwd, env).await {
        Ok(probe) => probe,
        Err(e) => {
            tracing::warn!(
                provider_id = provider_id,
                "provider options probe failed: {}",
                logs::redact_sensitive(&e.to_string())
            );
            return Ok(pinned_catalog);
        }
    };

    let fallback_current_model_id = pinned_catalog
        .as_ref()
        .and_then(ModelCatalog::default_model_id);
    let Some(models_value) = crate::api::provider_catalog::runtime_probe_models_payload(
        provider_id,
        &probe,
        fallback_current_model_id,
    ) else {
        return Ok(pinned_catalog);
    };
    if let Some(models) = build_model_catalog(&models_value) {
        let mut value = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace.id.0,
            "installed": true,
            "probe_ok": true,
            "supports_load": false,
            "auth_required": false,
            "models": models_value,
            "probed_at": chrono::Utc::now().to_rfc3339(),
        });
        value = redact_json_value(value);
        state.providers.options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value,
            },
        );
        return Ok(Some(models));
    }

    Ok(pinned_catalog)
}

#[cfg(test)]
pub(crate) async fn load_provider_model_catalog(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<Option<ModelCatalog>, String> {
    let install_target =
        crate::execution_effective::effective_install_target(state.as_ref(), workspace.id)
            .await
            .map_err(|err| {
                format!("workspace execution settings unavailable for provider options: {err:#}")
            })?;
    load_provider_model_catalog_for_install_target(state, workspace, provider_id, install_target)
        .await
}

pub(crate) async fn load_provider_model_catalog_for_execution_environment(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    execution_environment: ctx_core::models::ExecutionEnvironment,
) -> Result<Option<ModelCatalog>, String> {
    let install_target = crate::execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        workspace.id,
        execution_environment,
    )
    .await
    .map_err(|err| {
        format!("workspace execution settings unavailable for provider options: {err:#}")
    })?;
    load_provider_model_catalog_for_install_target(state, workspace, provider_id, install_target)
        .await
}
