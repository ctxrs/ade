use super::*;

pub(crate) async fn refresh_provider_matrix(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MatrixRefreshResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let outcome = state
        .providers
        .refresh_provider_matrix_from_local_sources(&state.core.data_root)
        .await;
    crate::daemon::providers::refresh_provider_statuses(state.as_ref())
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
        provider_count: outcome.matrix.providers.len(),
        generated_at: outcome.matrix.generated_at,
        source: outcome.source.as_str().to_string(),
        degraded: outcome.degraded,
        last_error: outcome
            .last_error
            .map(|value| logs::redact_sensitive(&value)),
    }))
}

fn dev_tools_enabled() -> bool {
    std::env::var("CTX_DEV_MODE")
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
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

    let results = state
        .providers
        .restart_all_provider_adapters(&reason, mode)
        .await
        .into_iter()
        .map(|result| DevRestartProvidersResult {
            provider_id: result.provider_id,
            status: result.status.as_str().to_string(),
            message: result.message,
        })
        .collect();

    Ok(Json(DevRestartProvidersResp {
        mode: mode.as_str().to_string(),
        results,
    }))
}
