use super::*;

pub(in crate::api) async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    const CACHE_TTL: Duration = Duration::from_secs(30);
    const VERIFY_TTL: Duration = Duration::from_secs(30 * 60);

    if provider_id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }

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

    let managed = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
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
    let source_config =
        harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
            .await
            .ok();
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
    attach_verify_cache(&mut out, verify_entry.as_ref(), VERIFY_TTL);
    Ok(Json(out))
}

pub(in crate::api) async fn verify_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    if provider_id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let ws_id = parse_workspace_id(&ws_id)?;

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

    let install_target = install_target_for_workspace(&state, workspace.id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let managed = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
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
    if !known {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ));
    }
    let provider_status = provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        &provider_id,
        install_target,
    )
    .await;

    let checked_at = Utc::now().to_rfc3339();
    let mut status = "ok".to_string();
    let mut auth_required = Some(false);
    let mut message: Option<String> = None;
    let mut endpoint_status = HarnessEndpointVerificationStatus::Valid;
    let source_config =
        harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
            .await
            .ok();
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());
    let mut selected_endpoint_id: Option<String> =
        selected_endpoint_from_harness_config(source_config);

    if !provider_status_is_usable(&provider_status) {
        status = "error".to_string();
        auth_required = Some(false);
        message = Some(
            provider_status_unusable_reason(&provider_status)
                .unwrap_or_else(|| "provider not ready for use".to_string()),
        );
        endpoint_status = HarnessEndpointVerificationStatus::Error;
    } else if let Some(endpoint) = selected_endpoint
        .as_ref()
        .filter(|endpoint| endpoint_supports_model_catalog_verify(endpoint))
    {
        match harness_sources::refresh_provider_endpoint_model_catalog(
            &state.core.data_root,
            &provider_id,
            &endpoint.id,
        )
        .await
        {
            Ok(refreshed_endpoint) => {
                selected_endpoint_id = Some(refreshed_endpoint.id.clone());
                let (next_status, next_auth, next_message, next_endpoint_status) =
                    endpoint_catalog_verify_outcome(&refreshed_endpoint);
                status = next_status;
                auth_required = next_auth;
                message = next_message;
                endpoint_status = next_endpoint_status;
            }
            Err(err) => {
                let msg = logs::redact_sensitive(&err.to_string());
                let (classified, auth, endpoint_verify) = classify_probe_error(&msg);
                status = classified.to_string();
                auth_required = auth;
                message = Some(msg);
                endpoint_status = endpoint_verify;
            }
        }

        if status == "ok" {
            match prepare_provider_runtime_probe(
                &state,
                &workspace,
                &provider_id,
                selected_endpoint_id.clone(),
            )
            .await
            {
                Ok(prepared) => {
                    selected_endpoint_id = prepared.selected_endpoint_id;
                    if let Err(err) = probe_crp_runtime_launch(
                        &provider_id,
                        prepared.command,
                        prepared.args,
                        prepared.cwd,
                        prepared.env,
                    )
                    .await
                    {
                        let msg = logs::redact_sensitive(&err.to_string());
                        let (next_status, next_auth, next_message, next_endpoint_status) =
                            endpoint_catalog_runtime_probe_failure(msg, endpoint_status);
                        status = next_status;
                        auth_required = next_auth;
                        message = next_message;
                        endpoint_status = next_endpoint_status;
                    }
                }
                Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
                Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
                    let msg = logs::redact_sensitive(&err);
                    let (next_status, next_auth, next_message, next_endpoint_status) =
                        endpoint_catalog_runtime_probe_failure(msg, endpoint_status);
                    status = next_status;
                    auth_required = next_auth;
                    message = next_message;
                    endpoint_status = next_endpoint_status;
                }
            }
        }
    } else {
        match prepare_provider_runtime_probe(
            &state,
            &workspace,
            &provider_id,
            selected_endpoint_id.clone(),
        )
        .await
        {
            Ok(prepared) => {
                selected_endpoint_id = prepared.selected_endpoint_id;
                let probe = probe_crp_models(
                    &provider_id,
                    prepared.command,
                    prepared.args,
                    prepared.cwd,
                    prepared.env,
                )
                .await;
                if let Err(err) = probe {
                    let msg = logs::redact_sensitive(&err.to_string());
                    let (classified, auth, endpoint_verify) = classify_probe_error(&msg);
                    status = classified.to_string();
                    auth_required = auth;
                    message = Some(msg);
                    endpoint_status = endpoint_verify;
                }
            }
            Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
            Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
                let msg = logs::redact_sensitive(&err);
                let (classified, auth, endpoint_verify) = classify_probe_error(&msg);
                status = classified.to_string();
                auth_required = auth;
                message = Some(msg);
                endpoint_status = endpoint_verify;
            }
        }
    }

    if let Some(endpoint_id) = selected_endpoint_id.as_ref() {
        let _ = harness_sources::mark_endpoint_verification(
            &state.core.data_root,
            &provider_id,
            endpoint_id,
            endpoint_status,
            message.clone(),
        )
        .await;
    }

    let resp = ProviderAuthCheckResp {
        provider_id: provider_id.clone(),
        workspace_id: ws_id.0.to_string(),
        status: status.clone(),
        auth_required,
        checked_at: Some(checked_at),
        message: message.clone(),
    };
    let verify_value =
        redact_json_value(serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null));
    let cache_key = workspace_provider_cache_key(ws_id, install_target, &provider_id);
    state.providers.verify_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: verify_value,
        },
    );

    Ok(Json(resp))
}

pub(in crate::api) async fn authenticate_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
    req: Option<Json<AuthenticateProviderReq>>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    if provider_id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let ws_id = parse_workspace_id(&ws_id)?;
    let method_id = req.and_then(|value| value.0.method_id);
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

    let probe_context = probe::provider_auth_context_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        &provider_id,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": err,
            })),
        )
    })?;
    if probe_context.source.source_kind == HarnessSourceKind::Endpoint {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "selected source is endpoint; update endpoint key/config directly instead of interactive authenticate",
            })),
        ));
    }

    let install_target = install_target_for_workspace(&state, workspace.id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let adapter =
        ensure_provider_adapter_for_target(state.as_ref(), &provider_id, install_target).await;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let checked_at = Utc::now().to_rfc3339();
    let result = adapter
        .authenticate_session(
            format!("auth-{}", uuid::Uuid::new_v4()),
            probe_context.cwd,
            probe_context.env,
            method_id,
            event_tx,
        )
        .await;

    let resp = match result {
        Ok(()) => ProviderAuthCheckResp {
            provider_id: provider_id.clone(),
            workspace_id: ws_id.0.to_string(),
            status: "ok".to_string(),
            auth_required: Some(false),
            checked_at: Some(checked_at),
            message: None,
        },
        Err(err) => {
            let msg = logs::redact_sensitive(&err.to_string());
            let (status, auth_required, _) = classify_probe_error(&msg);
            ProviderAuthCheckResp {
                provider_id: provider_id.clone(),
                workspace_id: ws_id.0.to_string(),
                status: status.to_string(),
                auth_required,
                checked_at: Some(checked_at),
                message: Some(msg),
            }
        }
    };
    let verify_value =
        redact_json_value(serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null));
    let cache_key = workspace_provider_cache_key(ws_id, install_target, &provider_id);
    state.providers.verify_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: verify_value,
        },
    );

    Ok(Json(resp))
}

pub(in crate::api) async fn install_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<InstallStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let target = crate::installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": e.to_string()
            })),
        )
    })?;

    let (install_id, _) =
        provider_launch_install::start_provider_install(&state, &id, target).await?;

    Ok(Json(InstallStartResponse {
        provider_id: id,
        install_id,
        target,
    }))
}

pub(in crate::api) async fn install_all_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<InstallStartResponse>>, StatusCode> {
    let target = crate::installer::parse_install_target(query.target.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let installs = provider_launch_install::start_all_provider_installs(&state, target).await;
    Ok(Json(
        installs
            .into_iter()
            .map(|(provider_id, install_id)| InstallStartResponse {
                provider_id,
                install_id,
                target,
            })
            .collect(),
    ))
}

pub(in crate::api) async fn get_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_polling_info(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct GetInstallStatusesReq {
    pub(in crate::api) install_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct InstallStatusBatchItem {
    pub(in crate::api) install_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::api) info: Option<InstallInfo>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct GetInstallStatusesResp {
    pub(in crate::api) installs: Vec<InstallStatusBatchItem>,
}

pub(in crate::api) async fn get_install_statuses(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GetInstallStatusesReq>,
) -> Result<Json<GetInstallStatusesResp>, (StatusCode, Json<ApiErrorResp>)> {
    let install_ids = req
        .install_ids
        .into_iter()
        .map(|raw| {
            let parsed = uuid::Uuid::parse_str(raw.trim()).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("invalid install id: {raw}"),
                    }),
                )
            })?;
            Ok(InstallId::from(parsed))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut installs = Vec::with_capacity(install_ids.len());
    for install_id in install_ids {
        let info = state.get_install_polling_info(install_id).await;
        installs.push(InstallStatusBatchItem {
            install_id: install_id.to_string(),
            info,
        });
    }

    Ok(Json(GetInstallStatusesResp { installs }))
}

pub(in crate::api) async fn cancel_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .cancel_install(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(in crate::api) async fn list_install_events(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<Vec<InstallProgressEvent>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_events(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(in crate::api) async fn install_stream_sse(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, axum::Error>>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let Some(sender) = state.get_install_sender(install_id).await else {
        return Err(StatusCode::NOT_FOUND);
    };

    let history = state
        .get_install_events(install_id)
        .await
        .unwrap_or_default();
    let initial = futures::stream::iter(history.into_iter().map(|ev| {
        let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
        Ok::<_, axum::Error>(SseEvent::default().event("progress").data(payload))
    }));

    let live = futures::stream::unfold(sender.subscribe(), move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    return Some((Ok(SseEvent::default().event("progress").data(payload)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let stream = initial.chain(live);

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
