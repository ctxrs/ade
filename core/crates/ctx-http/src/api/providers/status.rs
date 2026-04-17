use super::*;

use anyhow::Context;
use ctx_core::ids::WorkspaceId;

use crate::execution_effective;
#[allow(unused_imports)]
pub(crate) use crate::provider_launch::status::{
    apply_target_aware_provider_status, provider_status_for_target,
};

pub(super) async fn providers_statuses_response(
    state: &Arc<AppState>,
    target: InstallTarget,
    include_matrix_providers: bool,
) -> Vec<ProviderStatus> {
    let managed = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let mut seen = HashSet::new();
    let mut provider_ids = Vec::new();
    {
        let map = state.providers.statuses.lock().await;
        for provider_id in map.keys() {
            if !crate::provider_matrix::is_user_facing_harness_id(&matrix, provider_id) {
                continue;
            }
            if seen.insert(provider_id.clone()) {
                provider_ids.push(provider_id.clone());
            }
        }
    }
    if include_matrix_providers {
        for entry in &matrix.providers {
            if entry.kind != crate::provider_matrix::ProviderMatrixEntryKind::Harness {
                continue;
            }
            if seen.insert(entry.id.clone()) {
                provider_ids.push(entry.id.clone());
            }
        }
    }
    let mut out = Vec::with_capacity(provider_ids.len());
    for provider_id in provider_ids {
        out.push(
            provider_status_for_target(state.as_ref(), &managed, &matrix, &provider_id, target)
                .await,
        );
    }

    let show_fake = std::env::var("CTX_SHOW_FAKE_PROVIDER")
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
        .unwrap_or(false);
    for status in &mut out {
        if status.provider_id == "fake" {
            status.details.insert(
                "ui_hidden".into(),
                if show_fake { "false" } else { "true" }.into(),
            );
        }
        status
            .details
            .insert("install_target".into(), target.as_str().to_string());
        if let Some(bytes) =
            installer::managed_install_download_size_bytes(&matrix, &status.provider_id, target)
        {
            status
                .details
                .insert("install_download_size_bytes".into(), bytes.to_string());
        }
        if let Some(install_id) = state
            .find_running_install(&status.provider_id, Some(target))
            .await
        {
            status
                .details
                .insert("install_running".into(), "true".into());
            status
                .details
                .insert("install_id".into(), install_id.to_string());
        }
    }
    out
}

pub(super) async fn install_target_for_workspace(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> anyhow::Result<InstallTarget> {
    execution_effective::effective_install_target(state.as_ref(), workspace_id)
        .await
        .with_context(|| {
            format!(
                "loading execution settings for workspace {}",
                workspace_id.0
            )
        })
}

pub(super) fn workspace_execution_settings_error_json(
    error: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": format!("failed to load workspace execution settings: {error:#}"),
        })),
    )
}

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
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let target = installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let managed = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let known = {
        let map = state.providers.statuses.lock().await;
        map.contains_key(&id) || crate::provider_matrix::get_entry(&matrix, &id).is_some()
    };
    if !known {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("provider not found: {id}")
            })),
        ));
    }
    let mut status =
        provider_status_for_target(state.as_ref(), &managed, &matrix, &id, target).await;
    if let Some(bytes) =
        installer::managed_install_download_size_bytes(&matrix, &status.provider_id, target)
    {
        status
            .details
            .insert("install_download_size_bytes".into(), bytes.to_string());
    }
    if let Some(install_id) = state.find_running_install(&id, Some(target)).await {
        status
            .details
            .insert("install_running".into(), "true".into());
        status
            .details
            .insert("install_id".into(), install_id.to_string());
    }
    Ok(Json(status))
}

pub(crate) async fn get_provider_usage(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<provider_usage::ProviderUsageSnapshot>, (StatusCode, Json<serde_json::Value>)> {
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let refresh = query.refresh.unwrap_or(false);
    let snapshot = if !refresh {
        let cache = state.providers.usage_cache.lock().await;
        cache.get(&id).cloned()
    } else {
        None
    };
    let snapshot = match snapshot {
        Some(snapshot) => snapshot,
        None => {
            let env = if id == "codex" {
                provider_accounts::codex_env_for_active_account(&state.core.data_root)
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": e.to_string()
                            })),
                        )
                    })?
            } else {
                HashMap::new()
            };
            provider_usage::refresh_provider_usage_for(state.as_ref(), &id, env)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": e.to_string()
                        })),
                    )
                })?
        }
    };
    Ok(Json(snapshot))
}
