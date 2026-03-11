use std::path::Path as StdPath;

use crate::harness_sources;
use crate::harness_sources::HarnessSourceKind;

pub(super) fn endpoint_selection_is_active(
    config: &harness_sources::HarnessProviderSourceConfig,
) -> bool {
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

pub(super) async fn provider_has_active_auth_config(
    data_root: &StdPath,
    provider_id: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> bool {
    if let Some(config) = source_config {
        if endpoint_selection_is_active(config) {
            return true;
        }
    }
    if provider_id == "codex" {
        return crate::provider_accounts::codex_has_active_auth(data_root)
            .await
            .unwrap_or(false);
    }
    match crate::provider_accounts::subscription_env_for_active_account(data_root, provider_id)
        .await
    {
        Ok(env) => !env.is_empty(),
        Err(_) => false,
    }
}

pub(super) fn provider_auth_mode(
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
