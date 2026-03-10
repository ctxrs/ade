use super::*;

pub(crate) async fn install_lsp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LspInstallStartResponse>, StatusCode> {
    if !installer::is_supported_managed_lsp_server(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let install_key = format!("lsp:{id}");
    let (install_id, started_new) = state.start_install(install_key, None).await;
    if started_new {
        let state2 = state.clone();
        let server_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_lsp_server_with_progress(
                state2.clone(),
                install_id,
                server_id.clone(),
            )
            .await
            {
                tracing::error!("lsp install failed ({server_id}): {e:#}");
            }
        });
    }

    Ok(Json(LspInstallStartResponse {
        server_id: id,
        install_id,
    }))
}

pub(crate) async fn refresh_provider_matrix(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MatrixRefreshResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::provider_matrix::invalidate_matrix_cache(&state.providers.matrix_cache).await;
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    installer::refresh_provider_statuses(state.as_ref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to refresh provider statuses: {e:#}"),
                }),
            )
        })?;
    Ok(Json(MatrixRefreshResponse {
        provider_count: matrix.providers.len(),
        generated_at: matrix.generated_at,
    }))
}

fn dev_tools_enabled() -> bool {
    std::env::var("CTX_DEV_MODE")
        .ok()
        .map(|raw| {
            let v = raw.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

fn parse_restart_mode(value: &str) -> Option<ProviderRestartMode> {
    match value.trim().to_lowercase().as_str() {
        "immediate" => Some(ProviderRestartMode::Immediate),
        "drain" => Some(ProviderRestartMode::Drain),
        _ => None,
    }
}

pub(crate) async fn dev_restart_providers(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DevRestartProvidersReq>,
) -> Result<Json<DevRestartProvidersResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !dev_tools_enabled() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "dev tools are disabled".to_string(),
            }),
        ));
    }

    let Some(mode) = parse_restart_mode(&req.mode) else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mode must be 'immediate' or 'drain'".to_string(),
            }),
        ));
    };
    let reason = req
        .reason
        .unwrap_or_else(|| format!("dev restart ({})", mode.as_str()));

    let adapters = {
        let providers = state.providers.adapters.lock().await;
        let mut adapters = providers
            .iter()
            .map(|(id, adapter)| (id.clone(), Arc::clone(adapter)))
            .collect::<Vec<_>>();
        drop(providers);
        let target_adapters = state.providers.target_adapters.lock().await;
        adapters.extend(
            target_adapters
                .iter()
                .map(|(id, adapter)| (id.clone(), Arc::clone(adapter))),
        );
        adapters
    };

    let mut results = Vec::with_capacity(adapters.len());
    for (provider_id, adapter) in adapters {
        match adapter.restart(&reason, mode).await {
            Ok(()) => results.push(DevRestartProvidersResult {
                provider_id,
                status: "ok".to_string(),
                message: None,
            }),
            Err(err) => {
                let message = err.to_string();
                let status = if message.to_lowercase().contains("does not support") {
                    "unsupported"
                } else {
                    "error"
                };
                results.push(DevRestartProvidersResult {
                    provider_id,
                    status: status.to_string(),
                    message: Some(message),
                });
            }
        }
    }

    Ok(Json(DevRestartProvidersResp {
        mode: mode.as_str().to_string(),
        results,
    }))
}
