use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::Json;

use crate::daemon::AppState;
use crate::installer;
use crate::installs::{InstallId, InstallStateKind, InstallTarget};
use crate::provider_install_contract;

use super::status::provider_status_for_target;

struct DeferredBulkProviderInstall {
    provider_id: String,
    install_id: InstallId,
}

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
    let mut deferred_acp_repairs = Vec::new();
    for entry in &matrix.providers {
        if !installer::is_supported_managed_provider_for_target(&matrix, &entry.id, target) {
            continue;
        }
        let id = entry.id.as_str();
        let issue = provider_install_contract::provider_install_viability_issue(
            &state.core.data_root,
            &managed,
            &matrix,
            id,
            target,
        );
        if let Some(issue) = issue {
            if should_defer_acp_provider_until_bridge_repair(&matrix, id, target, &issue) {
                deferred_acp_repairs.push(id.to_string());
            }
            continue;
        }
        if let Some(install_id) =
            start_bulk_provider_install_if_needed(state, &managed, &matrix, id, target).await
        {
            out.push((id.to_string(), install_id));
        }
    }

    if deferred_acp_repairs.is_empty() {
        return out;
    }

    let refreshed_managed = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let mut deferred_queue = Vec::new();
    let mut deferred_bridge_install_id = None;
    for provider_id in deferred_acp_repairs {
        let running_bridge_install_id = state
            .find_running_install("acp-crp-bridge", Some(target))
            .await;
        let issue = provider_install_contract::provider_install_viability_issue(
            &state.core.data_root,
            &refreshed_managed,
            &matrix,
            &provider_id,
            target,
        );
        let should_queue_after_repair = match issue {
            None => true,
            Some(ref issue)
                if should_defer_acp_provider_until_bridge_repair(
                    &matrix,
                    &provider_id,
                    target,
                    issue,
                ) =>
            {
                if running_bridge_install_id.is_some() {
                    true
                } else {
                    let latest_managed = installer::load_agent_server_config(&state.core.data_root)
                        .await
                        .unwrap_or_default();
                    provider_install_contract::provider_install_viability_issue(
                        &state.core.data_root,
                        &latest_managed,
                        &matrix,
                        &provider_id,
                        target,
                    )
                    .is_none()
                }
            }
            Some(_) => false,
        };
        if !should_queue_after_repair {
            continue;
        }

        if let Some(bridge_install_id) = running_bridge_install_id {
            deferred_bridge_install_id.get_or_insert(bridge_install_id);
        }
        let install_id = queue_deferred_bulk_provider_install(
            state,
            &provider_id,
            target,
            running_bridge_install_id,
            &mut deferred_queue,
        )
        .await;
        out.push((provider_id, install_id));
    }
    spawn_deferred_bulk_provider_installs(
        state.clone(),
        deferred_bridge_install_id,
        deferred_queue,
        target,
    );
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

async fn start_bulk_provider_install_if_needed(
    state: &Arc<AppState>,
    managed: &installer::AgentServerConfigFile,
    matrix: &crate::provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> Option<InstallId> {
    if let Some(install_id) = state.find_running_install(provider_id, Some(target)).await {
        return Some(install_id);
    }

    let status = provider_status_for_target(state, managed, matrix, provider_id, target).await;
    if should_skip_install_for_healthy_provider(&status) {
        return None;
    }

    let (install_id, started_new) = state
        .start_install(provider_id.to_string(), Some(target))
        .await;
    if started_new {
        seed_running_prerequisite_progress(state, managed, matrix, provider_id, target, install_id)
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

    Some(install_id)
}

fn should_defer_acp_provider_until_bridge_repair(
    matrix: &crate::provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
    issue: &provider_install_contract::ProviderInstallViabilityIssue,
) -> bool {
    issue.code == "acp_bridge_invalid"
        && crate::daemon::is_acp_provider_id(provider_id)
        && installer::is_supported_managed_provider_for_target(matrix, "acp-crp-bridge", target)
}

async fn queue_deferred_bulk_provider_install(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
    bridge_install_id: Option<InstallId>,
    queue: &mut Vec<DeferredBulkProviderInstall>,
) -> InstallId {
    let (install_id, started_new) = state
        .start_install(provider_id.to_string(), Some(target))
        .await;
    if started_new {
        if let Some(bridge_install_id) = bridge_install_id {
            let _ = state
                .register_install_progress_mirror(bridge_install_id, install_id)
                .await;
        }
        queue.push(DeferredBulkProviderInstall {
            provider_id: provider_id.to_string(),
            install_id,
        });
    }
    install_id
}

fn spawn_deferred_bulk_provider_installs(
    state: Arc<AppState>,
    bridge_install_id: Option<InstallId>,
    deferred_installs: Vec<DeferredBulkProviderInstall>,
    target: InstallTarget,
) {
    if deferred_installs.is_empty() {
        return;
    }
    tokio::spawn(async move {
        if let Some(bridge_install_id) = bridge_install_id {
            wait_for_install_to_finish(&state, bridge_install_id).await;
        }
        for deferred_install in deferred_installs {
            if let Err(e) = installer::install_provider_with_progress(
                state.clone(),
                deferred_install.install_id,
                deferred_install.provider_id.clone(),
                target,
            )
            .await
            {
                tracing::error!(
                    "provider install failed ({}): {e:#}",
                    deferred_install.provider_id
                );
            }
        }
    });
}

async fn wait_for_install_to_finish(state: &Arc<AppState>, install_id: InstallId) {
    loop {
        let Some(info) = state.get_install_info(install_id).await else {
            return;
        };
        if !matches!(info.state, InstallStateKind::Running) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
