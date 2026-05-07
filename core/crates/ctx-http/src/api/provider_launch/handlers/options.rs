use super::*;

pub(in crate::api) async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    const CACHE_TTL: Duration = Duration::from_secs(30);
    const VERIFY_TTL: Duration = Duration::from_secs(30 * 60);

    let ws_id = parse_workspace_id(&ws_id)?;
    let install_target = install_target_for_workspace(&state, ws_id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let (managed, managed_config_error) =
        load_managed_agent_server_config_with_error(&state.core.data_root).await;
    let matrix = ctx_provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let known = {
        let map = state.providers.statuses.lock().await;
        map.contains_key(&provider_id)
            || ctx_provider_matrix::get_entry(&matrix, &provider_id).is_some()
    };
    let (source_config, source_config_error) =
        load_provider_source_config_with_error(&state.core.data_root, &provider_id).await;
    let skip_cached_config_surfaces =
        managed_config_error.is_some() || source_config_error.is_some();
    let cache = ProviderOptionsCacheSnapshot::load(
        &state,
        ws_id,
        install_target,
        &provider_id,
        skip_cached_config_surfaces,
    )
    .await;
    if let Some(out) = cache.fresh_authoritative_response(CACHE_TTL, VERIFY_TTL) {
        return Ok(Json(out));
    }
    if !known {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ));
    }

    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "failed to load workspace",
                })),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ))?;
    let preferred_model_id = load_workspace_preferred_model_id(&state, ws_id, &provider_id).await?;
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());

    if let Some(config_error) = managed_config_error.as_ref() {
        let mut raw_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": provider_auth_mode(false, source_config.as_ref()),
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        });
        attach_source_config(&mut raw_resp, source_config.as_ref());
        let out = finalize_provider_options_response(
            ProviderOptionsResponseContext {
                state: &state,
                provider_id: &provider_id,
                provider_status: None,
                selected_endpoint: None,
                cache: &cache,
                preferred_model_id: preferred_model_id.clone(),
            },
            raw_resp,
            false,
            VERIFY_TTL,
        )
        .await;
        return Ok(Json(out));
    }

    let provider_status = provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        &provider_id,
        install_target,
    )
    .await;

    if let Some(config_error) = source_config_error.as_ref() {
        let now = chrono::Utc::now();
        let raw_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.installed,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": provider_auth_mode(false, source_config.as_ref()),
            "probed_at": now.to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        });
        let out = finalize_provider_options_response(
            ProviderOptionsResponseContext {
                state: &state,
                provider_id: &provider_id,
                provider_status: Some(&provider_status),
                selected_endpoint: None,
                cache: &cache,
                preferred_model_id: preferred_model_id.clone(),
            },
            raw_resp,
            false,
            VERIFY_TTL,
        )
        .await;
        return Ok(Json(out));
    }

    let has_active_auth = match probe::provider_has_active_auth_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        &provider_id,
        source_config.as_ref(),
    )
    .await
    {
        Ok(value) => value,
        Err(config_error) => {
            let config_error = logs::redact_sensitive(&config_error);
            let now = chrono::Utc::now();
            let raw_resp = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.installed,
                "probe_ok": false,
                "supports_load": false,
                "auth_required": false,
                "has_active_auth": false,
                "auth_mode": "none",
                "probed_at": now.to_rfc3339(),
                "probe_error": config_error,
                "config_error": config_error,
            });
            let out = finalize_provider_options_response(
                ProviderOptionsResponseContext {
                    state: &state,
                    provider_id: &provider_id,
                    provider_status: Some(&provider_status),
                    selected_endpoint: None,
                    cache: &cache,
                    preferred_model_id: preferred_model_id.clone(),
                },
                raw_resp,
                false,
                VERIFY_TTL,
            )
            .await;
            return Ok(Json(out));
        }
    };
    let auth_mode = provider_auth_mode(has_active_auth, source_config.as_ref());

    if !provider_status_is_usable(&provider_status) {
        let mut raw_base_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.installed,
            "health": provider_status.health,
            "diagnostics": provider_status.diagnostics,
            "usability": provider_status.usability,
            "probe_ok": false,
            "probe_error": provider_status_unusable_reason(&provider_status)
                .unwrap_or_else(|| "provider not ready for use".to_string()),
            "has_active_auth": has_active_auth,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
        });
        attach_source_config(&mut raw_base_resp, source_config.as_ref());
        let out = finalize_provider_options_response(
            ProviderOptionsResponseContext {
                state: &state,
                provider_id: &provider_id,
                provider_status: Some(&provider_status),
                selected_endpoint: selected_endpoint.as_ref(),
                cache: &cache,
                preferred_model_id: preferred_model_id.clone(),
            },
            raw_base_resp,
            true,
            VERIFY_TTL,
        )
        .await;
        return Ok(Json(out));
    }

    let use_crp_probe = provider_supports_runtime_model_catalog(&provider_id);
    match provider_options_probe_plan(
        use_crp_probe,
        selected_endpoint
            .as_ref()
            .map(|endpoint| endpoint.id.as_str()),
    ) {
        ProviderOptionsProbePlan::EnvOnly => {
            let (probe_ok, auth_required, probe_error) =
                probe_provider_options_env(&state, &workspace, &provider_id).await;
            let now = chrono::Utc::now();
            let mut raw_resp = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.installed,
                "probe_ok": probe_ok,
                "supports_load": false,
                "auth_required": auth_required,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": now.to_rfc3339(),
            });
            if let Some(probe_error) = probe_error {
                raw_resp["probe_error"] = serde_json::json!(probe_error);
            }
            attach_source_config(&mut raw_resp, source_config.as_ref());
            let out = finalize_provider_options_response(
                ProviderOptionsResponseContext {
                    state: &state,
                    provider_id: &provider_id,
                    provider_status: Some(&provider_status),
                    selected_endpoint: selected_endpoint.as_ref(),
                    cache: &cache,
                    preferred_model_id: preferred_model_id.clone(),
                },
                raw_resp,
                true,
                VERIFY_TTL,
            )
            .await;
            return Ok(Json(out));
        }
        ProviderOptionsProbePlan::SelectedEndpointRuntimeLaunch(endpoint_id) => {
            let endpoint = selected_endpoint.as_ref().ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "selected endpoint missing from provider configuration",
                })),
            ))?;
            let now = chrono::Utc::now();
            let (probe_ok, auth_required, probe_error) = probe_selected_endpoint_runtime_launch(
                &state,
                &workspace,
                &provider_id,
                endpoint_id.to_string(),
            )
            .await?;
            let mut raw_resp = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.installed,
                "probe_ok": probe_ok,
                "supports_load": false,
                "auth_required": auth_required,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "models": endpoint_models_payload(&provider_id, endpoint, now),
                "probed_at": now.to_rfc3339(),
            });
            if let Some(probe_error) = probe_error {
                raw_resp["probe_error"] = serde_json::json!(probe_error);
            }
            attach_source_config(&mut raw_resp, source_config.as_ref());
            let out = finalize_provider_options_response(
                ProviderOptionsResponseContext {
                    state: &state,
                    provider_id: &provider_id,
                    provider_status: Some(&provider_status),
                    selected_endpoint: selected_endpoint.as_ref(),
                    cache: &cache,
                    preferred_model_id: preferred_model_id.clone(),
                },
                raw_resp,
                true,
                VERIFY_TTL,
            )
            .await;
            return Ok(Json(out));
        }
        ProviderOptionsProbePlan::RuntimeModels => {}
    }

    let probe = probe_runtime_models_for_provider_options(&state, &workspace, &provider_id).await?;

    let mut raw_resp = match probe {
        Ok(probe) => {
            let fallback_current_model_id =
                subscription_models_payload_from_status(&provider_status).and_then(|models| {
                    models
                        .get("current_model_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                });
            let mut value = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.installed,
                "probe_ok": true,
                "supports_load": false,
                "auth_required": false,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            });
            if let Some(models) = runtime_probe_models_payload(
                &provider_id,
                &probe,
                fallback_current_model_id.as_deref(),
            ) {
                value["models"] = models;
            } else {
                let probed_at = value
                    .get("probed_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let catalog_source = probe.catalog_source.as_deref().unwrap_or("missing");
                let current_model_id = probe.current_model_id.as_deref().unwrap_or("missing");
                let model_count = probe.models.len();
                value = serde_json::json!({
                    "provider_id": provider_id,
                    "workspace_id": ws_id.0,
                    "installed": provider_status.installed,
                    "probe_ok": false,
                    "probe_error": format!(
                        "runtime_model_catalog_missing: provider={provider_id} catalog_source={catalog_source} current_model_id={current_model_id} model_count={model_count}"
                    ),
                    "auth_required": false,
                    "has_active_auth": has_active_auth,
                    "auth_mode": auth_mode,
                    "probed_at": probed_at,
                    "supports_load": false,
                });
            }
            value
        }
        Err(e) => {
            let probe_error = logs::redact_sensitive(&e.to_string());
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.installed,
                "probe_ok": false,
                "probe_error": probe_error,
                "auth_required": auth_required.unwrap_or(false),
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            })
        }
    };
    attach_source_config(&mut raw_resp, source_config.as_ref());
    let out = finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state: &state,
            provider_id: &provider_id,
            provider_status: Some(&provider_status),
            selected_endpoint: selected_endpoint.as_ref(),
            cache: &cache,
            preferred_model_id,
        },
        raw_resp,
        true,
        VERIFY_TTL,
    )
    .await;
    Ok(Json(out))
}

async fn probe_provider_options_env(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> (bool, bool, Option<String>) {
    match probe::provider_probe_env_for_workspace_runtime(state.as_ref(), workspace, provider_id)
        .await
    {
        Ok(_) => (true, false, None),
        Err(err) => {
            let probe_error = logs::redact_sensitive(&err);
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            (false, auth_required.unwrap_or(false), Some(probe_error))
        }
    }
}

async fn probe_selected_endpoint_runtime_launch(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    endpoint_id: String,
) -> Result<(bool, bool, Option<String>), (StatusCode, Json<serde_json::Value>)> {
    match prepare_provider_runtime_probe(state, workspace, provider_id, Some(endpoint_id)).await {
        Ok(prepared) => {
            match probe_crp_runtime_launch(
                provider_id,
                prepared.command,
                prepared.args,
                prepared.cwd,
                prepared.env,
            )
            .await
            {
                Ok(()) => Ok((true, false, None)),
                Err(err) => {
                    let probe_error = logs::redact_sensitive(&err.to_string());
                    let (_, auth_required, _) = classify_probe_error(&probe_error);
                    Ok((false, auth_required.unwrap_or(false), Some(probe_error)))
                }
            }
        }
        Err(PreparedProviderRuntimeProbeError::Route(err)) => Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
            let probe_error = logs::redact_sensitive(&err);
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            Ok((false, auth_required.unwrap_or(false), Some(probe_error)))
        }
    }
}

async fn probe_runtime_models_for_provider_options(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> Result<anyhow::Result<ctx_providers::crp::CrpModelsProbe>, (StatusCode, Json<serde_json::Value>)>
{
    match prepare_provider_runtime_probe(state, workspace, provider_id, None).await {
        Ok(prepared) => Ok(probe_crp_models(
            provider_id,
            prepared.command,
            prepared.args,
            prepared.cwd,
            prepared.env,
        )
        .await),
        Err(PreparedProviderRuntimeProbeError::Route(err)) => Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => Ok(Err(anyhow::anyhow!(err))),
    }
}
