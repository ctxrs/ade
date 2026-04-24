use super::*;
use ctx_core::provider_ids::LEGACY_CODEX_PROVIDER_ID;
use ctx_workspace_config as workspace_config;

fn parse_workspace_id(ws_id: &str) -> Result<WorkspaceId, (StatusCode, Json<serde_json::Value>)> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid workspace id",
            })),
        )
    })?))
}

pub(crate) async fn get_workspace_providers_bootstrap(
    State(state): State<Arc<AppState>>,
    Path(ws_id): Path<String>,
) -> Result<Json<ProvidersBootstrapResponse>, (StatusCode, Json<serde_json::Value>)> {
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
        })?;
    let Some(_workspace) = workspace else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ));
    };

    let install_target = status::install_target_for_workspace(&state, ws_id)
        .await
        .map_err(|error| status::workspace_execution_settings_error_json(&error))?;
    let store = state.store_for_workspace(ws_id).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!(
                    "failed to load workspace store: {}",
                    logs::redact_sensitive(&error.to_string())
                ),
            })),
        )
    })?;
    let preferred_model_by_provider = std::sync::Arc::new(
        workspace_config::load_preferred_new_session_models(&store)
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!(
                            "failed to load workspace provider model preferences: {}",
                            logs::redact_sensitive(&error.to_string())
                        ),
                    })),
                )
            })?,
    );

    let providers = status::providers_statuses_response(&state, install_target, true).await;
    let mut provider_options = HashMap::new();
    let mut provider_harness_config = HashMap::new();
    let ws_id_str = ws_id.0.to_string();
    let visible_providers = providers
        .iter()
        .filter(|provider| !provider.detail_flag("ui_hidden").unwrap_or(false))
        .cloned()
        .collect::<Vec<_>>();

    let per_provider =
        futures::stream::iter(visible_providers.into_iter().map(|provider_status| {
            let state = Arc::clone(&state);
            let ws_id = ws_id_str.clone();
            let preferred_model_by_provider = std::sync::Arc::clone(&preferred_model_by_provider);
            async move {
                let provider_id = provider_status.provider_id.clone();
                let source_config = harness_sources::get_provider_source_config(
                    &state.core.data_root,
                    &provider_id,
                )
                .await
                .ok();
                // Bootstrap is auth/config hydration only. It must stay substrate-agnostic and
                // never cross into workspace runtime preparation.
                let has_active_auth =
                    crate::api::provider_probe_auth::provider_has_active_auth_config_with_runtime_root(
                        &state.core.data_root,
                        None,
                        &provider_id,
                        source_config.as_ref(),
                    )
                    .await;
                let auth_mode = probe::provider_auth_mode(has_active_auth, source_config.as_ref());
                let (probe_ok, auth_required, probe_error) =
                    probe::bootstrap_provider_probe_summary(&provider_status, has_active_auth);

                let mut options = serde_json::json!({
                    "provider_id": provider_id,
                    "workspace_id": ws_id,
                    "supports_load": false,
                    "auth_required": auth_required,
                    "has_active_auth": has_active_auth,
                    "auth_mode": auth_mode,
                    "probe_ok": probe_ok,
                    "probed_at": chrono::Utc::now().to_rfc3339(),
                });
                if let Some(probe_error) = probe_error {
                    options["probe_error"] = serde_json::json!(probe_error);
                }
                if let Some(source) = source_config.as_ref() {
                    options["source"] =
                        serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
                }
                if let Some(endpoint) =
                    crate::api::provider_launch::selected_endpoint_record_from_harness_config(
                        source_config.as_ref(),
                    )
                {
                    options["models"] = crate::api::provider_launch::endpoint_models_payload(
                        &provider_id,
                        &endpoint,
                        chrono::Utc::now(),
                    );
                } else if let Some(models) =
                    crate::api::provider_launch::subscription_models_payload_from_status(
                        &provider_status,
                    )
                {
                    options["models"] = models;
                }
                if let Some(preferred_model_id) =
                    crate::provider_model_preferences::preferred_model_id_from_available_models(
                        preferred_model_by_provider.get(&provider_id).cloned(),
                        options.get("models"),
                    )
                {
                    options["preferred_model_id"] = serde_json::json!(preferred_model_id);
                }

                (provider_id, options, source_config)
            }
        }))
        .buffer_unordered(visible_provider_count_hint(providers.len()))
        .collect::<Vec<_>>()
        .await;

    for (provider_id, options, source_config) in per_provider {
        provider_options.insert(provider_id.clone(), options);
        if let Some(config) = source_config {
            provider_harness_config.insert(provider_id, config);
        }
    }

    if let Some(codex_options) = provider_options.get("codex-crp").cloned() {
        let mut legacy_options = codex_options;
        legacy_options["provider_id"] = serde_json::json!(LEGACY_CODEX_PROVIDER_ID);
        provider_options.insert(LEGACY_CODEX_PROVIDER_ID.to_string(), legacy_options);
    }
    if let Some(codex_config) = provider_harness_config.get("codex-crp").cloned() {
        let mut legacy_config = codex_config;
        super::project_harness_config_for_response(LEGACY_CODEX_PROVIDER_ID, &mut legacy_config);
        provider_harness_config.insert(LEGACY_CODEX_PROVIDER_ID.to_string(), legacy_config);
    }
    Ok(Json(ProvidersBootstrapResponse {
        providers,
        provider_options,
        provider_harness_config,
        codex_accounts: accounts::codex_accounts_response(&state).await,
        claude_accounts: accounts::claude_accounts_response(&state).await,
        gemini_accounts: accounts::gemini_accounts_response(&state).await,
        qwen_accounts: accounts::qwen_accounts_response(&state).await,
        kimi_accounts: accounts::kimi_accounts_response(&state).await,
        mistral_accounts: accounts::mistral_accounts_response(&state).await,
        copilot_accounts: accounts::copilot_accounts_response(&state).await,
        cursor_accounts: accounts::cursor_accounts_response(&state).await,
        amp_accounts: accounts::amp_accounts_response(&state).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!(
                        "failed to load amp accounts: {}",
                        logs::redact_sensitive(&e.to_string())
                    ),
                })),
            )
        })?,
    }))
}

fn visible_provider_count_hint(total_provider_count: usize) -> usize {
    total_provider_count.max(1)
}
