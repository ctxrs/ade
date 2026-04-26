use super::*;

use anyhow::Context;
use ctx_core::ids::WorkspaceId;
use ctx_core::provider_ids::CODEX_CRP_PROVIDER_ID;
use ctx_providers::adapters::{
    ProviderHealth, ProviderRecommendedAction, ProviderUsability, ProviderUsabilityStatus,
};

use crate::execution_effective;
#[allow(unused_imports)]
pub(crate) use crate::provider_launch::status::{
    apply_target_aware_provider_status, provider_status_for_target,
};

fn mark_provider_status_with_managed_config_error(status: &mut ProviderStatus, config_error: &str) {
    let reason = format!("managed provider config error: {config_error}");
    status.health = ProviderHealth::Error;
    status
        .details
        .insert("managed_config_error".into(), "true".into());
    status.details.insert(
        "managed_config_error_message".into(),
        config_error.to_string(),
    );
    if !status.diagnostics.iter().any(|value| value == &reason) {
        status.diagnostics.push(reason.clone());
    }
    status.usability = ProviderUsability {
        usable: false,
        status: ProviderUsabilityStatus::Blocked,
        reason_code: Some("managed_config_error".into()),
        reason: Some(reason),
        blocking_provider_ids: Vec::new(),
        recommended_action: ProviderRecommendedAction::ConfigureRuntime,
    };
}

async fn provider_status_without_target_bootstrap(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> ProviderStatus {
    if matches!(target, InstallTarget::Host) {
        return state
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
                health: ProviderHealth::Missing,
                diagnostics: vec![format!("provider not available: {provider_id}")],
                details: std::collections::HashMap::new(),
                usability: ProviderUsability::default(),
            });
    }

    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: false,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ProviderHealth::Missing,
        diagnostics: vec![format!(
            "provider not available for target '{}'",
            target.as_str()
        )],
        details: std::collections::HashMap::new(),
        usability: ProviderUsability::default(),
    }
}

pub(crate) async fn providers_statuses_response(
    state: &Arc<AppState>,
    target: InstallTarget,
    include_matrix_providers: bool,
) -> Vec<ProviderStatus> {
    let (managed, managed_config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
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
        let status = if managed_config_error.is_some() {
            provider_status_without_target_bootstrap(state, &provider_id, target).await
        } else {
            provider_status_for_target(state.as_ref(), &managed, &matrix, &provider_id, target)
                .await
        };
        out.push(status);
    }
    let legacy_aliases = out
        .iter()
        .filter_map(super::legacy_codex_status_alias)
        .collect::<Vec<_>>();
    out.extend(legacy_aliases);

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
        if let Some(config_error) = managed_config_error.as_deref() {
            mark_provider_status_with_managed_config_error(status, config_error);
        }
    }
    out
}

pub(crate) async fn install_target_for_workspace(
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
    let requested_id = id;
    let id = super::canonicalize_provider_id(&requested_id);
    let target = installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let (managed, managed_config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
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
    let mut status = if managed_config_error.is_some() {
        provider_status_without_target_bootstrap(&state, &id, target).await
    } else {
        provider_status_for_target(state.as_ref(), &managed, &matrix, &id, target).await
    };
    status.provider_id =
        super::project_provider_id_for_response(&requested_id, &status.provider_id);
    if let Some(config_error) = managed_config_error.as_deref() {
        mark_provider_status_with_managed_config_error(&mut status, config_error);
    }
    if let Some(bytes) = installer::managed_install_download_size_bytes(&matrix, &id, target) {
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

fn provider_usage_internal_error(
    error: impl std::fmt::Display,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": error.to_string()
        })),
    )
}

async fn provider_usage_env_for_request(
    state: &Arc<AppState>,
    provider_id: &str,
) -> Result<HashMap<String, String>, (StatusCode, Json<serde_json::Value>)> {
    if provider_id != CODEX_CRP_PROVIDER_ID {
        return Ok(HashMap::new());
    }

    let mut env = provider_accounts::codex_env_for_active_account(&state.core.data_root)
        .await
        .map_err(provider_usage_internal_error)?;
    let (cfg, config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        return Err(provider_usage_internal_error(config_error));
    }
    crate::installer::ensure_codex_cli_command_env_for_target(
        &mut env,
        &cfg,
        CODEX_CRP_PROVIDER_ID,
        Some(InstallTarget::Host),
    )
    .map_err(provider_usage_internal_error)?;
    Ok(env)
}

pub(crate) async fn get_provider_usage(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<provider_usage::ProviderUsageSnapshot>, (StatusCode, Json<serde_json::Value>)> {
    let requested_id = id;
    let id = super::canonicalize_provider_id(&requested_id);
    let refresh = query.refresh.unwrap_or(false);
    let env = provider_usage_env_for_request(&state, &id).await?;
    let snapshot = if !refresh {
        let cache = state.providers.usage_cache.lock().await;
        cache.get(&id).cloned()
    } else {
        None
    };
    let mut snapshot = match snapshot {
        Some(snapshot) => snapshot,
        None => provider_usage::refresh_provider_usage_for(state.as_ref(), &id, env)
            .await
            .map_err(provider_usage_internal_error)?,
    };
    snapshot.provider_id =
        super::project_provider_id_for_response(&requested_id, &snapshot.provider_id);
    Ok(Json(snapshot))
}
