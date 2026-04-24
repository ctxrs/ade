use std::path::Path as StdPath;

use ctx_core::provider_ids::{canonical_provider_id, CODEX_CRP_PROVIDER_ID};
use ctx_harness_sources as harness_sources;
use ctx_harness_sources::HarnessSourceKind;
use ctx_provider_accounts as provider_accounts;

pub fn endpoint_selection_is_active(config: &harness_sources::HarnessProviderSourceConfig) -> bool {
    if config.selected_source_kind != HarnessSourceKind::Endpoint {
        return false;
    }
    let Some(selected_endpoint_id) = config.selected_endpoint_id.as_deref() else {
        return false;
    };
    config
        .endpoints
        .iter()
        .any(|endpoint| endpoint.id == selected_endpoint_id)
}

pub async fn provider_has_active_auth_config(
    data_root: &StdPath,
    provider_id: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> bool {
    provider_has_active_auth_config_with_runtime_root(data_root, None, provider_id, source_config)
        .await
}

pub async fn provider_has_active_auth_config_with_runtime_root(
    data_root: &StdPath,
    runtime_data_root: Option<&StdPath>,
    provider_id: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> bool {
    let provider_id = canonical_provider_id(provider_id);
    if provider_id == "fake" {
        return true;
    }
    if let Some(config) = source_config {
        if endpoint_selection_is_active(config) {
            return true;
        }
    }
    if provider_id == CODEX_CRP_PROVIDER_ID {
        return match runtime_data_root {
            Some(runtime_root) => {
                provider_accounts::codex_has_active_auth_with_runtime_root(data_root, runtime_root)
                    .await
            }
            None => provider_accounts::codex_has_active_auth(data_root).await,
        }
        .unwrap_or(false);
    }
    let env = match runtime_data_root {
        Some(runtime_root) => {
            provider_accounts::subscription_env_for_active_account_with_runtime_root(
                data_root,
                runtime_root,
                provider_id,
            )
            .await
        }
        None => {
            provider_accounts::subscription_env_for_active_account(data_root, provider_id).await
        }
    };
    match env {
        Ok(env) => !env.is_empty(),
        Err(_) => false,
    }
}

pub fn provider_auth_mode(
    has_active_auth: bool,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> &'static str {
    if !has_active_auth {
        return "none";
    }
    if let Some(config) = source_config {
        if endpoint_selection_is_active(config) {
            return "endpoint";
        }
    }
    "subscription"
}
