use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ctx_managed_installs as installer;
use ctx_managed_installs::provider_install_contract;
use ctx_provider_install::install_state::{InstallId, InstallStateKind, InstallTarget};
use ctx_provider_matrix as provider_matrix;

use super::resolver::is_acp_provider_id;
use super::status::provider_status_for_target;
use crate::ProviderRuntimeHost;

#[async_trait]
pub trait ProviderInstallHost:
    installer::ManagedInstallHost + ProviderRuntimeHost + Send + Sync + 'static
{
    async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId>;
}

#[derive(Debug, Clone)]
pub struct StartProviderInstallError {
    pub message: String,
    pub code: Option<String>,
}

struct DeferredBulkProviderInstall {
    provider_id: String,
    install_id: InstallId,
}

async fn start_contract_readiness_dependencies<H>(
    state: &Arc<H>,
    managed: &installer::AgentServerConfigFile,
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
    install_id: InstallId,
) where
    H: ProviderInstallHost,
{
    let Ok(contract) = provider_install_contract::resolve_provider_install_contract(
        installer::ManagedInstallHost::data_root(state.as_ref()),
        managed,
        matrix,
        provider_id,
        target,
    ) else {
        return;
    };
    for dependency in contract.dependencies_for_role(
        provider_install_contract::ProviderInstallDependencyRoleKind::Readiness,
    ) {
        if dependency.satisfied {
            continue;
        }
        let (dependency_install_id, started_new) = state
            .start_install(dependency.provider_id.clone(), Some(dependency.target))
            .await;
        let _ = state
            .register_install_progress_mirror(dependency_install_id, install_id)
            .await;
        if !started_new {
            continue;
        }
        let state2 = state.clone();
        let dependency_provider_id = dependency.provider_id.clone();
        tokio::spawn(async move {
            if let Err(error) = installer::install_provider_with_progress(
                state2.clone(),
                dependency_install_id,
                dependency_provider_id.clone(),
                dependency.target,
            )
            .await
            {
                tracing::error!(
                    "provider dependency install failed ({dependency_provider_id}): {error:#}"
                );
            }
        });
    }
}

pub async fn start_provider_install<H>(
    state: &Arc<H>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<(InstallId, bool), StartProviderInstallError>
where
    H: ProviderInstallHost,
{
    let matrix = provider_matrix::load_matrix_cached(
        installer::ManagedInstallHost::data_root(state.as_ref()),
        state.provider_matrix_cache(),
    )
    .await;
    let managed = installer::load_agent_server_config(
        installer::ManagedInstallHost::data_root(state.as_ref()),
    )
        .await
        .unwrap_or_default();
    if !installer::is_supported_managed_provider_for_target(&matrix, provider_id, target) {
        return Err(StartProviderInstallError {
            message: format!(
                "unsupported provider for managed install target '{}': {provider_id}",
                target.as_str()
            ),
            code: None,
        });
    }
    if let Some(issue) = provider_install_contract::provider_install_viability_issue(
        installer::ManagedInstallHost::data_root(state.as_ref()),
        &managed,
        &matrix,
        provider_id,
        target,
    ) {
        return Err(StartProviderInstallError {
            message: issue.message,
            code: Some(issue.code.to_string()),
        });
    }

    let (install_id, started_new) = state.start_install(provider_id.to_string(), Some(target)).await;
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
        start_contract_readiness_dependencies(
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

pub async fn start_all_provider_installs<H>(
    state: &Arc<H>,
    target: InstallTarget,
) -> Vec<(String, InstallId)>
where
    H: ProviderInstallHost,
{
    let mut out = Vec::new();
    let matrix = provider_matrix::load_matrix_cached(
        installer::ManagedInstallHost::data_root(state.as_ref()),
        state.provider_matrix_cache(),
    )
    .await;
    let managed = installer::load_agent_server_config(
        installer::ManagedInstallHost::data_root(state.as_ref()),
    )
        .await
        .unwrap_or_default();
    let mut deferred_acp_repairs = Vec::new();
    for entry in &matrix.providers {
        if entry.kind != provider_matrix::ProviderMatrixEntryKind::Harness {
            continue;
        }
        if !installer::is_supported_managed_provider_for_target(&matrix, &entry.id, target) {
            continue;
        }
        let id = entry.id.as_str();
        if should_defer_acp_provider_until_stale_bridge_repair(&managed, &matrix, id, target) {
            deferred_acp_repairs.push(id.to_string());
            continue;
        }
        let issue = provider_install_contract::provider_install_viability_issue(
            installer::ManagedInstallHost::data_root(state.as_ref()),
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

    let mut deferred_bridge_install_id = state
        .find_running_install("acp-crp-bridge", Some(target))
        .await;
    if deferred_bridge_install_id.is_none() {
        if let Some(bridge_install_id) = start_bulk_provider_install_if_needed(
            state,
            &managed,
            &matrix,
            "acp-crp-bridge",
            target,
        )
        .await
        {
            deferred_bridge_install_id = Some(bridge_install_id);
            out.push(("acp-crp-bridge".to_string(), bridge_install_id));
        }
    }

    let refreshed_managed = installer::load_agent_server_config(
        installer::ManagedInstallHost::data_root(state.as_ref()),
    )
        .await
        .unwrap_or_default();
    let mut deferred_queue = Vec::new();
    for provider_id in deferred_acp_repairs {
        let issue = provider_install_contract::provider_install_viability_issue(
            installer::ManagedInstallHost::data_root(state.as_ref()),
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
                deferred_bridge_install_id.is_some()
            }
            Some(_) => false,
        };
        if !should_queue_after_repair {
            continue;
        }

        let install_id = queue_deferred_bulk_provider_install(
            state,
            &provider_id,
            target,
            deferred_bridge_install_id,
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

async fn seed_running_prerequisite_progress<H>(
    state: &Arc<H>,
    managed: &installer::AgentServerConfigFile,
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
    install_id: InstallId,
) where
    H: ProviderInstallHost,
{
    let Ok(contract) = provider_install_contract::resolve_provider_install_contract(
        installer::ManagedInstallHost::data_root(state.as_ref()),
        managed,
        matrix,
        provider_id,
        target,
    ) else {
        return;
    };
    for dependency in &contract.dependencies {
        if dependency.satisfied {
            continue;
        }
        let Some(prerequisite_install_id) = state
            .find_running_install(&dependency.provider_id, Some(dependency.target))
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
        .detail_flag("matrix_update_available")
        .unwrap_or(false);
    let dependency_update = status
        .detail_flag("managed_dependency_update_available")
        .unwrap_or(false);
    matrix_update || dependency_update
}

pub fn should_skip_install_for_healthy_provider(
    status: &ctx_providers::adapters::ProviderStatus,
) -> bool {
    status.installed
        && matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
        && status.details.get("ready_for_use").map(String::as_str) != Some("false")
        && !has_provider_update_available(status)
}

async fn start_bulk_provider_install_if_needed<H>(
    state: &Arc<H>,
    managed: &installer::AgentServerConfigFile,
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> Option<InstallId>
where
    H: ProviderInstallHost,
{
    if let Some(install_id) = state.find_running_install(provider_id, Some(target)).await {
        return Some(install_id);
    }

    let status =
        provider_status_for_target(state.as_ref(), managed, matrix, provider_id, target).await;
    if should_skip_install_for_healthy_provider(&status) {
        return None;
    }

    let (install_id, started_new) = state.start_install(provider_id.to_string(), Some(target)).await;
    if started_new {
        seed_running_prerequisite_progress(state, managed, matrix, provider_id, target, install_id)
            .await;
        start_contract_readiness_dependencies(
            state,
            managed,
            matrix,
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

    Some(install_id)
}

fn should_defer_acp_provider_until_bridge_repair(
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
    issue: &provider_install_contract::ProviderInstallViabilityIssue,
) -> bool {
    issue.code == "acp_bridge_invalid"
        && is_acp_provider_id(provider_id)
        && installer::is_supported_managed_provider_for_target(matrix, "acp-crp-bridge", target)
}

fn should_defer_acp_provider_until_stale_bridge_repair(
    managed: &installer::AgentServerConfigFile,
    matrix: &provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> bool {
    if !is_acp_provider_id(provider_id)
        || !installer::is_supported_managed_provider_for_target(matrix, "acp-crp-bridge", target)
    {
        return false;
    }
    match installer::resolve_runtime_provider_command_for_target(
        managed,
        "acp-crp-bridge",
        Some(target),
    ) {
        Ok(_) => false,
        Err(_) => matches!(
            installer::resolve_runtime_provider_command_for_target_repairable_managed(
                managed,
                "acp-crp-bridge",
                Some(target),
            ),
            Ok(None)
        ),
    }
}

async fn queue_deferred_bulk_provider_install<H>(
    state: &Arc<H>,
    provider_id: &str,
    target: InstallTarget,
    bridge_install_id: Option<InstallId>,
    queue: &mut Vec<DeferredBulkProviderInstall>,
) -> InstallId
where
    H: ProviderInstallHost,
{
    let (install_id, started_new) = state.start_install(provider_id.to_string(), Some(target)).await;
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

fn spawn_deferred_bulk_provider_installs<H>(
    state: Arc<H>,
    bridge_install_id: Option<InstallId>,
    deferred_installs: Vec<DeferredBulkProviderInstall>,
    target: InstallTarget,
) where
    H: ProviderInstallHost,
{
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

async fn wait_for_install_to_finish<H>(state: &Arc<H>, install_id: InstallId)
where
    H: ProviderInstallHost,
{
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
