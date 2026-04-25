use super::*;

pub(in crate::api) async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    const CACHE_TTL: Duration = Duration::from_secs(30);
    const VERIFY_TTL: Duration = Duration::from_secs(30 * 60);

    let requested_provider_id = provider_id;
    let provider_id = canonicalize_provider_id(&requested_provider_id);

    let ws_id = parse_workspace_id(&ws_id)?;
    let install_target = install_target_for_workspace(&state, ws_id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let cache_key = workspace_provider_cache_key(ws_id, install_target, &provider_id);
    let verify_entry: Option<(std::time::Instant, serde_json::Value)> = state
        .providers
        .verify_cache
        .lock()
        .await
        .get(&cache_key)
        .map(|c| (c.cached_at, c.value.clone()));
    let cached_entry: Option<(std::time::Instant, serde_json::Value)> = state
        .providers
        .options_cache
        .lock()
        .await
        .get(&cache_key)
        .map(|c| (c.cached_at, c.value.clone()));
    let authoritative_cached_entry = cached_entry
        .as_ref()
        .filter(|(_, value)| provider_options_cache_entry_is_authoritative(&provider_id, value));
    if let Some((cached_at, cached_value)) = authoritative_cached_entry {
        if cached_at.elapsed() < CACHE_TTL {
            let mut out = cached_value.clone();
            project_provider_id_field(&requested_provider_id, &mut out);
            attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
            return Ok(Json(out));
        }
    }
    let cached_models = authoritative_cached_entry
        .as_ref()
        .and_then(|(_, value)| value.get("models"))
        .cloned()
        .filter(|v| !v.is_null());
    let cached_modes = authoritative_cached_entry
        .as_ref()
        .and_then(|(_, value)| value.get("modes"))
        .cloned()
        .filter(|v| !v.is_null());

    let (managed, managed_config_error) =
        load_managed_agent_server_config_with_error(&state.core.data_root).await;
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let known = {
        let map = state.providers.statuses.lock().await;
        map.contains_key(&provider_id)
            || crate::provider_matrix::get_entry(&matrix, &provider_id).is_some()
    };
    let (source_config, source_config_error) =
        load_provider_source_config_with_error(&state.core.data_root, &provider_id).await;
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

    let provider_status = provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        &provider_id,
        install_target,
    )
    .await;
    let has_active_auth = probe::provider_has_active_auth_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        &provider_id,
        source_config.as_ref(),
    )
    .await;
    let auth_mode = provider_auth_mode(has_active_auth, source_config.as_ref());
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());

    if let Some(config_error) = managed_config_error.as_ref() {
        let mut raw_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": has_active_auth,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        });
        if let Some(source) = source_config.as_ref() {
            raw_resp["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
        }
        inject_preferred_model_id(&mut raw_resp, preferred_model_id.clone());
        let resp = redact_json_value(raw_resp);
        state.providers.options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value: resp.clone(),
            },
        );
        let mut out = resp;
        project_provider_id_field(&requested_provider_id, &mut out);
        attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
        return Ok(Json(out));
    }

    if let Some(config_error) = source_config_error.as_ref() {
        let now = chrono::Utc::now();
        let mut raw_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.installed,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": has_active_auth,
            "auth_mode": auth_mode,
            "probed_at": now.to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        });
        attach_static_provider_models_and_modes(
            &state,
            &mut raw_resp,
            &provider_id,
            &provider_status,
            None,
            cached_models.clone(),
            cached_modes.clone(),
        )
        .await;
        inject_preferred_model_id(&mut raw_resp, preferred_model_id.clone());

        let resp = redact_json_value(raw_resp);
        state.providers.options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value: resp.clone(),
            },
        );
        let mut out = resp;
        project_provider_id_field(&requested_provider_id, &mut out);
        attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
        return Ok(Json(out));
    }

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
        if let Some(source) = source_config.as_ref() {
            raw_base_resp["source"] =
                serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
        }
        attach_static_provider_models_and_modes(
            &state,
            &mut raw_base_resp,
            &provider_id,
            &provider_status,
            selected_endpoint.as_ref(),
            cached_models.clone(),
            cached_modes.clone(),
        )
        .await;
        inject_preferred_model_id(&mut raw_base_resp, preferred_model_id.clone());
        let base_resp = redact_json_value(raw_base_resp);
        state.providers.options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value: base_resp.clone(),
            },
        );
        let mut out = base_resp;
        project_provider_id_field(&requested_provider_id, &mut out);
        attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
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
                match probe::provider_probe_env_for_workspace_runtime(
                    state.as_ref(),
                    &workspace,
                    &provider_id,
                )
                .await
                {
                    Ok(_) => (true, false, None),
                    Err(err) => {
                        let probe_error = logs::redact_sensitive(&err);
                        let (_, auth_required, _) = classify_probe_error(&probe_error);
                        (false, auth_required.unwrap_or(false), Some(probe_error))
                    }
                };
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
            if let Some(source) = source_config.as_ref() {
                raw_resp["source"] =
                    serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
            }
            attach_static_provider_models_and_modes(
                &state,
                &mut raw_resp,
                &provider_id,
                &provider_status,
                selected_endpoint.as_ref(),
                cached_models.clone(),
                cached_modes.clone(),
            )
            .await;
            inject_preferred_model_id(&mut raw_resp, preferred_model_id.clone());

            let resp = redact_json_value(raw_resp);
            state.providers.options_cache.lock().await.insert(
                cache_key,
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value: resp.clone(),
                },
            );

            let mut out = resp;
            project_provider_id_field(&requested_provider_id, &mut out);
            attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
            return Ok(Json(out));
        }
        ProviderOptionsProbePlan::SelectedEndpointRuntimeLaunch(endpoint_id) => {
            let ws = state
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
            let endpoint = selected_endpoint.as_ref().ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "selected endpoint missing from provider configuration",
                })),
            ))?;
            let now = chrono::Utc::now();
            let mut probe_ok = true;
            let mut auth_required = false;
            let mut probe_error: Option<String> = None;
            match prepare_provider_runtime_probe(
                &state,
                &ws,
                &provider_id,
                Some(endpoint_id.to_string()),
            )
            .await
            {
                Ok(prepared) => {
                    if let Err(err) = probe_crp_runtime_launch(
                        &provider_id,
                        prepared.command,
                        prepared.args,
                        prepared.cwd,
                        prepared.env,
                    )
                    .await
                    {
                        let probe_error_value = logs::redact_sensitive(&err.to_string());
                        let (_, next_auth_required, _) = classify_probe_error(&probe_error_value);
                        probe_ok = false;
                        auth_required = next_auth_required.unwrap_or(false);
                        probe_error = Some(probe_error_value);
                    }
                }
                Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
                Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
                    let probe_error_value = logs::redact_sensitive(&err);
                    let (_, next_auth_required, _) = classify_probe_error(&probe_error_value);
                    probe_ok = false;
                    auth_required = next_auth_required.unwrap_or(false);
                    probe_error = Some(probe_error_value);
                }
            }
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
            if let Some(source) = source_config.as_ref() {
                raw_resp["source"] =
                    serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
            }
            attach_static_provider_models_and_modes(
                &state,
                &mut raw_resp,
                &provider_id,
                &provider_status,
                selected_endpoint.as_ref(),
                cached_models.clone(),
                cached_modes.clone(),
            )
            .await;
            inject_preferred_model_id(&mut raw_resp, preferred_model_id.clone());
            let resp = redact_json_value(raw_resp);
            state.providers.options_cache.lock().await.insert(
                cache_key,
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value: resp.clone(),
                },
            );
            let mut out = resp;
            attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
            return Ok(Json(out));
        }
        ProviderOptionsProbePlan::RuntimeModels => {}
    }

    let ws = state
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

    let probe = match prepare_provider_runtime_probe(&state, &ws, &provider_id, None).await {
        Ok(prepared) => {
            probe_crp_models(
                &provider_id,
                prepared.command,
                prepared.args,
                prepared.cwd,
                prepared.env,
            )
            .await
        }
        Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => Err(anyhow::anyhow!(err)),
    };

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
    if let Some(source) = source_config.as_ref() {
        raw_resp["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
    }
    attach_static_provider_models_and_modes(
        &state,
        &mut raw_resp,
        &provider_id,
        &provider_status,
        selected_endpoint.as_ref(),
        cached_models,
        cached_modes,
    )
    .await;
    inject_preferred_model_id(&mut raw_resp, preferred_model_id);

    let resp = redact_json_value(raw_resp);
    state.providers.options_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: resp.clone(),
        },
    );

    let mut out = resp;
    project_provider_id_field(&requested_provider_id, &mut out);
    attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
    Ok(Json(out))
}
