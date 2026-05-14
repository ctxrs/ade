use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessProviderSourceConfig;
use ctx_observability::logs;
use ctx_provider_runtime::model_preferences::preferred_model_id_from_available_models;
use ctx_provider_runtime::provider_auth::{
    provider_auth_mode, provider_has_active_auth_config_with_runtime_root,
    selected_endpoint_record_from_harness_config,
};
use ctx_provider_runtime::provider_launch::models::{
    endpoint_models_payload, subscription_models_payload_from_status,
};
use ctx_provider_runtime::provider_usability::{
    provider_status_is_usable, provider_status_unusable_reason,
};
use ctx_providers::adapters::ProviderStatus;

use crate::daemon::DaemonState;

pub(crate) async fn build_bootstrap_options(
    state: &Arc<DaemonState>,
    ws_id: WorkspaceId,
    provider_status: ProviderStatus,
    preferred_model_id: Option<String>,
) -> (
    String,
    serde_json::Value,
    Option<HarnessProviderSourceConfig>,
) {
    let provider_id = provider_status.provider_id.clone();
    let (source_config, source_config_error) =
        ctx_provider_runtime::provider_launch::config::load_provider_source_config_with_error(
            &state.core.data_root,
            &provider_id,
        )
        .await;
    let (has_active_auth, auth_mode, auth_config_error) = provider_auth_summary(
        state,
        &provider_id,
        source_config.as_ref(),
        &source_config_error,
    )
    .await;
    let (mut probe_ok, mut auth_required, mut probe_error) =
        bootstrap_provider_probe_summary(&provider_status, has_active_auth);
    if let Some(config_error) = auth_config_error.as_ref() {
        probe_ok = false;
        auth_required = false;
        probe_error = Some(config_error.clone());
    }

    let mut options = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": ws_id.0.to_string(),
        "supports_load": false,
        "auth_required": auth_required,
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "probe_ok": probe_ok,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(probe_error) = probe_error {
        options["probe_error"] = serde_json::json!(probe_error);
    }
    if let Some(config_error) = source_config_error.as_ref().or(auth_config_error.as_ref()) {
        options["probe_ok"] = serde_json::json!(false);
        options["probe_error"] = serde_json::json!(config_error);
        options["config_error"] = serde_json::json!(config_error);
    }
    append_model_options(
        &provider_id,
        &provider_status,
        source_config.as_ref(),
        &mut options,
    );
    if let Some(preferred_model_id) =
        preferred_model_id_from_available_models(preferred_model_id, options.get("models"))
    {
        options["preferred_model_id"] = serde_json::json!(preferred_model_id);
    }

    if let Some(source) = source_config.as_ref() {
        options["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
    }

    (provider_id, options, source_config)
}

pub(crate) fn visible_provider_count_hint(total_provider_count: usize) -> usize {
    total_provider_count.max(1)
}

async fn provider_auth_summary(
    state: &Arc<DaemonState>,
    provider_id: &str,
    source_config: Option<&HarnessProviderSourceConfig>,
    source_config_error: &Option<String>,
) -> (bool, &'static str, Option<String>) {
    // Bootstrap is auth/config hydration only. It must stay substrate-agnostic and
    // never cross into workspace runtime preparation.
    if source_config_error.is_some() {
        return (false, "none", None);
    }

    match provider_has_active_auth_config_with_runtime_root(
        &state.core.data_root,
        None,
        provider_id,
        source_config,
    )
    .await
    {
        Ok(has_active_auth) => (
            has_active_auth,
            provider_auth_mode(has_active_auth, source_config),
            None,
        ),
        Err(err) => (false, "none", Some(logs::redact_sensitive(&err))),
    }
}

fn bootstrap_provider_probe_summary(
    provider_status: &ProviderStatus,
    has_active_auth: bool,
) -> (bool, bool, Option<String>) {
    if !provider_status_is_usable(provider_status) {
        return (
            false,
            false,
            Some(
                provider_status_unusable_reason(provider_status)
                    .unwrap_or_else(|| "provider not ready for use".to_string()),
            ),
        );
    }

    (true, !has_active_auth, None)
}

fn append_model_options(
    provider_id: &str,
    provider_status: &ProviderStatus,
    source_config: Option<&HarnessProviderSourceConfig>,
    options: &mut serde_json::Value,
) {
    if let Some(endpoint) = selected_endpoint_record_from_harness_config(source_config) {
        options["models"] = endpoint_models_payload(provider_id, &endpoint, chrono::Utc::now());
    } else if let Some(models) = subscription_models_payload_from_status(provider_status) {
        options["models"] = models;
    }
}
