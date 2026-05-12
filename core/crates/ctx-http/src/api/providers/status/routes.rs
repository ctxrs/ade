use super::aggregate::{
    decorate_provider_runtime_details, provider_status_without_target_bootstrap,
};
use super::*;

pub(crate) async fn list_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<ProviderStatus>>, StatusCode> {
    let target = installer::parse_install_target(query.target.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(
        providers_statuses_response(&state, target, false).await,
    ))
}

pub(crate) async fn get_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<ProviderStatus>, (StatusCode, Json<serde_json::Value>)> {
    let target = installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let (managed, managed_config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    ensure_known_provider(&state, &matrix, &id).await?;

    let mut status = if managed_config_error.is_some() {
        provider_status_without_target_bootstrap(&state, &id, target).await
    } else {
        provider_status_for_target(state.as_ref(), &managed, &matrix, &id, target).await
    };
    decorate_provider_runtime_details(
        &state,
        &matrix,
        managed_config_error.as_deref(),
        target,
        &mut status,
    )
    .await;
    Ok(Json(status))
}

async fn ensure_known_provider(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    provider_id: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if state
        .providers
        .is_known_provider_id(matrix, provider_id)
        .await
    {
        return Ok(());
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": format!("provider not found: {provider_id}")
        })),
    ))
}
