use super::*;

pub(in crate::api) async fn verify_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
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
    if !known {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ));
    }
    let checked_at = Utc::now().to_rfc3339();
    let mut status = "ok".to_string();
    let mut auth_required = Some(false);
    let mut message: Option<String> = None;
    let mut endpoint_status = HarnessEndpointVerificationStatus::Valid;
    let (source_config, source_config_error) =
        load_provider_source_config_with_error(&state.core.data_root, &provider_id).await;
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());
    let mut selected_endpoint_id: Option<String> =
        selected_endpoint_from_harness_config(source_config);

    if let Some(config_error) = managed_config_error {
        let resp = ProviderAuthCheckResp {
            provider_id: provider_id.clone(),
            workspace_id: ws_id.0.to_string(),
            status: "error".to_string(),
            auth_required: Some(false),
            checked_at: Some(checked_at.clone()),
            message: Some(config_error),
        };
        return Ok(Json(resp));
    }

    if let Some(config_error) = source_config_error {
        let resp = ProviderAuthCheckResp {
            provider_id: provider_id.clone(),
            workspace_id: ws_id.0.to_string(),
            status: "error".to_string(),
            auth_required: Some(false),
            checked_at: Some(checked_at.clone()),
            message: Some(config_error),
        };
        return Ok(Json(resp));
    }

    let provider_status = provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        &provider_id,
        install_target,
    )
    .await;

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
                    if let Err(err) = probe_crp_models(
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
