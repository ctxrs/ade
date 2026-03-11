use super::*;
use std::collections::BTreeSet;
use std::path::Path as StdPath;

use anyhow::Context;
use ctx_core::ids::WorkspaceId;

use crate::daemon::ensure_provider_adapter_for_target_with_cfg;
use crate::execution_effective;
use crate::provider_install_contract;

fn inspect_error_status(provider_id: &str, err: anyhow::Error) -> ProviderStatus {
    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: false,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Error,
        diagnostics: vec![err.to_string()],
        details: HashMap::new(),
    }
}

fn managed_targets_for_provider(
    managed: &installer::AgentServerConfigFile,
    provider_id: &str,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();

    if let Some(targets) = managed.managed_install_targets.get(provider_id) {
        for key in targets.keys() {
            if let Ok(target) = installer::parse_install_target(Some(key.as_str())) {
                out.insert(target.as_str().to_string());
            }
        }
    }
    if let Some(targets) = managed.managed_provider_targets.get(provider_id) {
        for key in targets.keys() {
            if let Ok(target) = installer::parse_install_target(Some(key.as_str())) {
                out.insert(target.as_str().to_string());
            }
        }
    }
    if let Some(target) = managed
        .providers
        .get(provider_id)
        .and_then(|entry| entry.managed.as_ref())
        .and_then(|meta| meta.target)
    {
        out.insert(target.as_str().to_string());
    }
    if let Some(target) = managed
        .managed_installs
        .get(provider_id)
        .and_then(|meta| meta.target)
    {
        out.insert(target.as_str().to_string());
    }

    out
}

fn synthesize_target_mismatch_status(
    managed: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Option<ProviderStatus> {
    let runtime_available = match installer::resolve_runtime_provider_command_for_target(
        managed,
        provider_id,
        Some(target),
    ) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => true,
    };
    if runtime_available {
        return None;
    }

    let available_targets = managed_targets_for_provider(managed, provider_id);
    if available_targets.is_empty() {
        return None;
    }

    let requested_target = target.as_str();
    let mut details = HashMap::new();
    details.insert("install_target".into(), requested_target.to_string());
    details.insert("target_mismatch".into(), "true".into());
    details.insert(
        "available_managed_targets".into(),
        available_targets
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(","),
    );
    if available_targets.len() == 1 {
        if let Some(target_value) = available_targets.iter().next() {
            details.insert("managed_target".into(), target_value.clone());
        }
    }

    let diagnostic = if available_targets.contains(requested_target) {
        format!(
            "provider is not installed for target '{}'; configure a valid runtime command or reinstall it for that target",
            requested_target
        )
    } else if available_targets.len() == 1 {
        let available_target = available_targets.iter().next().cloned().unwrap_or_default();
        format!(
            "provider is installed for target '{}' but not for target '{}'",
            available_target, requested_target
        )
    } else {
        format!(
            "provider is not installed for target '{}'; available managed targets: {}",
            requested_target,
            available_targets.into_iter().collect::<Vec<_>>().join(", ")
        )
    };

    Some(ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: false,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Missing,
        diagnostics: vec![diagnostic],
        details,
    })
}

pub(super) fn apply_target_aware_provider_status(
    status: &mut ProviderStatus,
    managed: &installer::AgentServerConfigFile,
    target: InstallTarget,
) {
    installer::apply_managed_install_details_for_target(status, managed, Some(target));
    installer::apply_install_target_status(status, target);
}

pub(crate) async fn provider_status_for_target(
    state: &Arc<AppState>,
    managed: &installer::AgentServerConfigFile,
    matrix: &crate::provider_matrix::ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> ProviderStatus {
    let mut status =
        if let Some(status) = synthesize_target_mismatch_status(managed, provider_id, target) {
            status
        } else if matches!(target, InstallTarget::Host) {
            state
                .providers
                .statuses
                .lock()
                .await
                .get(provider_id)
                .cloned()
                .unwrap_or_else(|| ProviderStatus {
                    provider_id: provider_id.to_string(),
                    installed: false,
                    detected_path: None,
                    version: None,
                    capabilities: None,
                    health: ctx_providers::adapters::ProviderHealth::Missing,
                    diagnostics: vec![format!("provider not available: {provider_id}")],
                    details: HashMap::new(),
                })
        } else {
            let adapter = ensure_provider_adapter_for_target_with_cfg(
                state.as_ref(),
                managed,
                provider_id,
                target,
            )
            .await;
            match adapter.inspect().await {
                Ok(status) => status,
                Err(err) => inspect_error_status(provider_id, err),
            }
        };
    status
        .details
        .insert("install_target".into(), target.as_str().to_string());
    apply_target_aware_provider_status(&mut status, managed, target);
    if let Some(entry) = crate::provider_matrix::get_entry(matrix, provider_id) {
        crate::provider_matrix::apply_matrix_to_status(
            &state.core.data_root,
            managed,
            entry,
            &mut status,
        )
        .await;
    }
    status
}

pub(super) fn apply_install_viability_details(
    status: &mut ProviderStatus,
    data_root: &StdPath,
    managed: &installer::AgentServerConfigFile,
    matrix: &crate::provider_matrix::ProviderMatrix,
    target: InstallTarget,
) {
    let install_viability = provider_install_contract::provider_install_viability_issue(
        data_root,
        managed,
        matrix,
        &status.provider_id,
        target,
    );
    status.details.insert(
        "install_supported".into(),
        if installer::is_supported_managed_provider_for_target(matrix, &status.provider_id, target)
            && install_viability.is_none()
        {
            "true".into()
        } else {
            "false".into()
        },
    );
    if let Some(issue) = install_viability {
        status
            .details
            .insert("install_blocked".into(), "true".into());
        status
            .details
            .insert("install_blocked_code".into(), issue.code.to_string());
        status
            .details
            .insert("install_blocked_reason".into(), issue.message.clone());
        if !status
            .diagnostics
            .iter()
            .any(|value| value == &issue.message)
        {
            status.diagnostics.push(issue.message);
        }
    } else {
        status.details.remove("install_blocked");
        status.details.remove("install_blocked_code");
        status.details.remove("install_blocked_reason");
    }
}

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
        out.push(provider_status_for_target(state, &managed, &matrix, &provider_id, target).await);
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
        apply_install_viability_details(status, &state.core.data_root, &managed, &matrix, target);
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
    let mut status = provider_status_for_target(&state, &managed, &matrix, &id, target).await;
    apply_install_viability_details(
        &mut status,
        &state.core.data_root,
        &managed,
        &matrix,
        target,
    );
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
            provider_usage::refresh_provider_usage_for(&state, &id, env)
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
