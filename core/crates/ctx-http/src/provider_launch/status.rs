use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Context;
use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;
use ctx_providers::adapters::ProviderStatus;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::installer;
use crate::installs::InstallTarget;

use super::resolver::ensure_provider_adapter_for_target_with_cfg;

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

pub(crate) fn apply_target_aware_provider_status(
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

pub(crate) fn workspace_execution_settings_error_json(
    error: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": format!("failed to load workspace execution settings: {error:#}"),
        })),
    )
}
