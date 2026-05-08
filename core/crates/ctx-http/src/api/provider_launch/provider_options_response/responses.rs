use super::*;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_runtime::provider_launch::models::{
    endpoint_models_payload, subscription_models_payload_from_status,
};
use ctx_providers::adapters::ProviderStatus;

fn attach_source_config(
    value: &mut serde_json::Value,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) {
    if let Some(source) = source_config {
        value["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
    }
}

pub(in crate::api::provider_launch) struct ProviderOptionsResponseBase<'a> {
    pub(in crate::api::provider_launch) provider_id: &'a str,
    pub(in crate::api::provider_launch) workspace_id: WorkspaceId,
    pub(in crate::api::provider_launch) provider_status: &'a ProviderStatus,
    pub(in crate::api::provider_launch) has_active_auth: bool,
    pub(in crate::api::provider_launch) auth_mode: &'a str,
    pub(in crate::api::provider_launch) source_config:
        Option<&'a harness_sources::HarnessProviderSourceConfig>,
}

pub(in crate::api::provider_launch) fn config_error_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    installed: Option<bool>,
    auth_mode: &str,
    config_error: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = match installed {
        Some(installed) => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace_id.0,
            "installed": installed,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        }),
        None => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace_id.0,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        }),
    };
    attach_source_config(&mut response, source_config);
    response
}

pub(in crate::api::provider_launch) fn unusable_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": workspace_id.0,
        "installed": provider_status.installed,
        "health": provider_status.health,
        "diagnostics": provider_status.diagnostics,
        "usability": provider_status.usability,
        "probe_ok": false,
        "probe_error": provider_status_unusable_reason(provider_status)
            .unwrap_or_else(|| "provider not ready for use".to_string()),
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    attach_source_config(&mut response, source_config);
    response
}

pub(in crate::api::provider_launch) fn env_probe_provider_options_response(
    base: ProviderOptionsResponseBase<'_>,
    probe_ok: bool,
    auth_required: bool,
    probe_error: Option<String>,
) -> serde_json::Value {
    let mut response = serde_json::json!({
        "provider_id": base.provider_id,
        "workspace_id": base.workspace_id.0,
        "installed": base.provider_status.installed,
        "probe_ok": probe_ok,
        "supports_load": false,
        "auth_required": auth_required,
        "has_active_auth": base.has_active_auth,
        "auth_mode": base.auth_mode,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(probe_error) = probe_error {
        response["probe_error"] = serde_json::json!(probe_error);
    }
    attach_source_config(&mut response, base.source_config);
    response
}

pub(in crate::api::provider_launch) fn selected_endpoint_runtime_launch_options_response(
    base: ProviderOptionsResponseBase<'_>,
    endpoint: &HarnessEndpointRecord,
    probe_ok: bool,
    auth_required: bool,
    probe_error: Option<String>,
) -> serde_json::Value {
    let now = chrono::Utc::now();
    let mut response = serde_json::json!({
        "provider_id": base.provider_id,
        "workspace_id": base.workspace_id.0,
        "installed": base.provider_status.installed,
        "probe_ok": probe_ok,
        "supports_load": false,
        "auth_required": auth_required,
        "has_active_auth": base.has_active_auth,
        "auth_mode": base.auth_mode,
        "models": endpoint_models_payload(base.provider_id, endpoint, now),
        "probed_at": now.to_rfc3339(),
    });
    if let Some(probe_error) = probe_error {
        response["probe_error"] = serde_json::json!(probe_error);
    }
    attach_source_config(&mut response, base.source_config);
    response
}

pub(in crate::api::provider_launch) fn runtime_models_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    probe: anyhow::Result<ctx_providers::crp::CrpModelsProbe>,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = match probe {
        Ok(probe) => {
            let fallback_current_model_id =
                subscription_models_payload_from_status(provider_status).and_then(|models| {
                    models
                        .get("current_model_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                });
            let mut value = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": workspace_id.0,
                "installed": provider_status.installed,
                "probe_ok": true,
                "supports_load": false,
                "auth_required": false,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            });
            if let Some(models) = runtime_probe_models_payload(
                provider_id,
                &probe,
                fallback_current_model_id.as_deref(),
            ) {
                value["models"] = models;
            } else {
                let probed_at = value
                    .get("probed_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let catalog_source = probe.catalog_source.as_deref().unwrap_or("missing");
                let current_model_id = probe.current_model_id.as_deref().unwrap_or("missing");
                let model_count = probe.models.len();
                value = serde_json::json!({
                    "provider_id": provider_id,
                    "workspace_id": workspace_id.0,
                    "installed": provider_status.installed,
                    "probe_ok": false,
                    "probe_error": format!(
                        "runtime_model_catalog_missing: provider={provider_id} catalog_source={catalog_source} current_model_id={current_model_id} model_count={model_count}"
                    ),
                    "auth_required": false,
                    "has_active_auth": has_active_auth,
                    "auth_mode": auth_mode,
                    "probed_at": probed_at,
                    "supports_load": false,
                });
            }
            value
        }
        Err(err) => {
            let probe_error = logs::redact_sensitive(&err.to_string());
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": workspace_id.0,
                "installed": provider_status.installed,
                "probe_ok": false,
                "probe_error": probe_error,
                "auth_required": auth_required.unwrap_or(false),
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            })
        }
    };
    attach_source_config(&mut response, source_config);
    response
}
