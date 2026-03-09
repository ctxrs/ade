use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;

use crate::daemon::AppState;
use crate::installer;
use crate::installs::{InstallId, InstallTarget};
use crate::provider_install_contract;

use super::status::provider_status_for_target;

pub(crate) async fn start_provider_install(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<(InstallId, bool), (StatusCode, Json<serde_json::Value>)> {
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let managed = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    if !installer::is_supported_managed_provider_for_target(&matrix, provider_id, target) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "unsupported provider for managed install target '{}': {provider_id}",
                    target.as_str()
                )
            })),
        ));
    }
    if let Some(issue) = provider_install_contract::provider_install_viability_issue(
        &state.core.data_root,
        &managed,
        &matrix,
        provider_id,
        target,
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": issue.message,
                "code": issue.code,
            })),
        ));
    }

    let (install_id, started_new) = state
        .start_install(provider_id.to_string(), Some(target))
        .await;
    if started_new {
        seed_running_prerequisite_progress(
            state,
            &managed,
            &matrix,
            provider_id,
            target,
            install_id,
        )
        .await;
        let state2 = state.clone();
        let provider_id = provider_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = installer::install_provider_with_progress(
                state2.clone(),
                install_id,
                provider_id.clone(),
                target,
            )
            .await
            {
                tracing::error!("provider install failed ({provider_id}): {e:#}");
            }
        });
    }

    Ok((install_id, started_new))
}

pub(crate) async fn start_all_provider_installs(
    state: &Arc<AppState>,
    target: InstallTarget,
) -> Vec<(String, InstallId)> {
    let mut out = Vec::new();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let managed = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    for entry in &matrix.providers {
        if !installer::is_supported_managed_provider_for_target(&matrix, &entry.id, target) {
            continue;
        }
        let id = entry.id.as_str();
        if provider_install_contract::provider_install_viability_issue(
            &state.core.data_root,
            &managed,
            &matrix,
            id,
            target,
        )
        .is_some()
        {
            continue;
        }
        if let Some(install_id) = state.find_running_install(id, Some(target)).await {
            out.push((id.to_string(), install_id));
            continue;
        }

        let st = provider_status_for_target(state, &managed, &matrix, id, target).await;
        if should_skip_install_for_healthy_provider(&st) {
            continue;
        }

        let (install_id, started_new) = state.start_install(id.to_string(), Some(target)).await;
        if started_new {
            seed_running_prerequisite_progress(state, &managed, &matrix, id, target, install_id)
                .await;
            let state2 = state.clone();
            let provider_id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = installer::install_provider_with_progress(
                    state2.clone(),
                    install_id,
                    provider_id.clone(),
                    target,
                )
                .await
                {
                    tracing::error!("provider install failed ({provider_id}): {e:#}");
                }
            });
        }
        out.push((id.to_string(), install_id));
    }
    out
}

async fn seed_running_prerequisite_progress(
    state: &Arc<AppState>,
    managed: &installer::AgentServerConfigFile,
    matrix: &crate::provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
    install_id: InstallId,
) {
    let Ok(contract) = provider_install_contract::resolve_provider_install_contract(
        &state.core.data_root,
        managed,
        matrix,
        provider_id,
        target,
    ) else {
        return;
    };
    for prerequisite in &contract.prerequisites {
        let Some(prerequisite_install_id) = state
            .find_running_install(prerequisite.provider_id, Some(target))
            .await
        else {
            continue;
        };
        let _ = state
            .register_install_progress_mirror(prerequisite_install_id, install_id)
            .await;
    }
}

fn has_provider_update_available(status: &ctx_providers::adapters::ProviderStatus) -> bool {
    let matrix_update = status
        .details
        .get("matrix_update_available")
        .map(|value| value == "true")
        .unwrap_or(false);
    let dependency_update = status
        .details
        .get("managed_dependency_update_available")
        .map(|value| value == "true")
        .unwrap_or(false);
    matrix_update || dependency_update
}

pub(crate) fn should_skip_install_for_healthy_provider(
    status: &ctx_providers::adapters::ProviderStatus,
) -> bool {
    status.installed
        && matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
        && !has_provider_update_available(status)
}
