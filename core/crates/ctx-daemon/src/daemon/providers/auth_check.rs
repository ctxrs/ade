use std::sync::Arc;

use chrono::Utc;
use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessEndpointVerificationStatus;
use ctx_observability::logs;
use ctx_provider_runtime::provider_launch::models::{
    endpoint_catalog_runtime_probe_failure, endpoint_catalog_verify_outcome,
};
use ctx_provider_runtime::provider_launch::options::endpoint_supports_model_catalog_verify;
use ctx_provider_runtime::provider_launch::probe_error::classify_probe_error;
use ctx_provider_runtime::provider_usability::{
    provider_status_is_usable, provider_status_unusable_reason,
};
use serde::Serialize;

use crate::daemon::providers::{
    authenticate_provider_for_workspace_runtime, install_target_for_workspace,
    load_provider_launch_config_snapshot, mark_provider_endpoint_verification,
    refresh_provider_endpoint_model_catalog, ProviderLaunchConfigError,
    ProviderWorkspaceAuthenticationError,
};
use crate::daemon::DaemonState;

mod cache;
mod outcome;
mod probe;
mod workspace;

use cache::store_provider_auth_check_cache;
use outcome::{config_error_snapshot, ProviderVerifyOutcome};
use probe::apply_auth_verification_probe;
use workspace::load_workspace;

#[derive(Debug)]
pub enum ProviderAuthCheckError {
    WorkspaceLoad,
    WorkspaceNotFound,
    ExecutionSettings(anyhow::Error),
    ProviderLaunchConfig(ProviderLaunchConfigError),
    Verify(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderAuthCheckSnapshot {
    pub provider_id: String,
    pub workspace_id: String,
    pub status: String,
    pub auth_required: Option<bool>,
    pub checked_at: Option<String>,
    pub message: Option<String>,
}

pub async fn authenticate_provider_for_workspace(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
    method_id: Option<String>,
) -> Result<ProviderAuthCheckSnapshot, ProviderAuthCheckError> {
    let workspace = load_workspace(state, workspace_id).await?;
    let auth =
        authenticate_provider_for_workspace_runtime(state, &workspace, provider_id, method_id)
            .await
            .map_err(|error| match error {
                ProviderWorkspaceAuthenticationError::ExecutionSettings(error) => {
                    ProviderAuthCheckError::ExecutionSettings(error)
                }
                ProviderWorkspaceAuthenticationError::Verify(error) => {
                    ProviderAuthCheckError::Verify(error)
                }
            })?;

    let snapshot = match auth.error_message {
        None => ProviderAuthCheckSnapshot {
            provider_id: provider_id.to_string(),
            workspace_id: workspace_id.0.to_string(),
            status: "ok".to_string(),
            auth_required: Some(false),
            checked_at: Some(auth.checked_at),
            message: None,
        },
        Some(message) => {
            let (status, auth_required, _) = classify_probe_error(&message);
            ProviderAuthCheckSnapshot {
                provider_id: provider_id.to_string(),
                workspace_id: workspace_id.0.to_string(),
                status: status.to_string(),
                auth_required,
                checked_at: Some(auth.checked_at),
                message: Some(message),
            }
        }
    };
    store_provider_auth_check_cache(
        state,
        workspace_id,
        auth.install_target,
        provider_id,
        &snapshot,
    )
    .await;
    Ok(snapshot)
}

pub async fn verify_provider_for_workspace(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<ProviderAuthCheckSnapshot, ProviderAuthCheckError> {
    let workspace = load_workspace(state, workspace_id).await?;
    let install_target = install_target_for_workspace(state, workspace.id)
        .await
        .map_err(ProviderAuthCheckError::ExecutionSettings)?;
    let launch_config = load_provider_launch_config_snapshot(state, provider_id).await;
    launch_config
        .ensure_known_provider(state, provider_id)
        .await
        .map_err(ProviderAuthCheckError::ProviderLaunchConfig)?;
    let checked_at = Utc::now().to_rfc3339();
    let selected_endpoint = launch_config.selected_endpoint_record();
    let mut selected_endpoint_id = launch_config.selected_endpoint_id();

    if let Some(config_error) = launch_config.managed_config_error.as_ref() {
        return Ok(config_error_snapshot(
            provider_id,
            workspace_id,
            &checked_at,
            config_error.clone(),
        ));
    }

    if let Some(config_error) = launch_config.source_config_error.as_ref() {
        return Ok(config_error_snapshot(
            provider_id,
            workspace_id,
            &checked_at,
            config_error.clone(),
        ));
    }

    let provider_status = launch_config
        .provider_status(state, provider_id, install_target)
        .await;
    let mut outcome = ProviderVerifyOutcome::new(checked_at, selected_endpoint_id.take());

    if !provider_status_is_usable(&provider_status) {
        outcome.apply_unusable_provider(
            provider_status_unusable_reason(&provider_status)
                .unwrap_or_else(|| "provider not ready for use".to_string()),
        );
    } else if let Some(endpoint) = selected_endpoint
        .as_ref()
        .filter(|endpoint| endpoint_supports_model_catalog_verify(endpoint))
    {
        match refresh_provider_endpoint_model_catalog(state, provider_id, &endpoint.id).await {
            Ok(refreshed_endpoint) => {
                outcome.apply_endpoint_catalog_refresh(refreshed_endpoint);
            }
            Err(err) => {
                outcome.apply_classified_probe_error(logs::redact_sensitive(&err.to_string()));
            }
        }

        if outcome.is_ok() {
            apply_auth_verification_probe(state, &workspace, provider_id, &mut outcome).await?;
        }
    } else {
        apply_auth_verification_probe(state, &workspace, provider_id, &mut outcome).await?;
    }

    if let Some(endpoint_id) = outcome.selected_endpoint_id() {
        let _ = mark_provider_endpoint_verification(
            state,
            provider_id,
            endpoint_id,
            outcome.endpoint_status(),
            outcome.message().cloned(),
        )
        .await;
    }

    let snapshot = outcome.into_snapshot(provider_id, workspace_id);
    store_provider_auth_check_cache(state, workspace_id, install_target, provider_id, &snapshot)
        .await;
    Ok(snapshot)
}
