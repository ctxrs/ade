#[cfg(test)]
pub(super) use super::super::provider_probe_auth::endpoint_selection_is_active;
pub(super) use super::super::provider_probe_auth::provider_auth_mode;
#[cfg(test)]
pub(super) use super::super::provider_probe_auth::provider_has_active_auth_config;
use super::*;

pub(super) fn classify_probe_error(
    message: &str,
) -> (
    &'static str,
    Option<bool>,
    HarnessEndpointVerificationStatus,
) {
    let lower = message.to_ascii_lowercase();
    let auth_required = [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "authentication required",
        "auth required",
        "auth_required",
        "auth failed",
        "auth_failed",
        "auth error",
        "auth_error",
        "not authenticated",
        "not logged in",
        "login required",
        "sign in",
        "api key",
        "missing token",
        "invalid token",
        "expired token",
        "access token",
        "bearer token",
        "active account",
        "account env",
        "configure an active account",
    ];
    if auth_required.iter().any(|needle| lower.contains(needle)) {
        return (
            "auth_required",
            Some(true),
            HarnessEndpointVerificationStatus::Invalid,
        );
    }
    let protocol_error = [
        "invalid message",
        "invalid acp",
        "invalid crp",
        "models.list response",
        "models.list probe",
    ];
    if protocol_error.iter().any(|needle| lower.contains(needle)) {
        return (
            "error",
            Some(false),
            HarnessEndpointVerificationStatus::Error,
        );
    }
    if lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("network")
        || lower.contains("econn")
        || lower.contains("dns")
        || lower.contains("tls")
    {
        return (
            "network_error",
            Some(false),
            HarnessEndpointVerificationStatus::Error,
        );
    }
    (
        "error",
        Some(false),
        HarnessEndpointVerificationStatus::Error,
    )
}

pub(super) fn cache_key_matches_provider(cache_key: &str, provider_id: &str) -> bool {
    cache_key
        .rsplit('/')
        .next()
        .is_some_and(|key_provider| key_provider == provider_id)
}

pub(super) async fn invalidate_provider_probe_caches(state: &Arc<AppState>, provider_id: &str) {
    state
        .providers
        .options_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
    state
        .providers
        .verify_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
}

pub(super) async fn bootstrap_provider_probe_summary(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    install_target: InstallTarget,
    provider_status: &ProviderStatus,
    provider_id: &str,
) -> (bool, bool, Option<String>) {
    if !crate::provider_usability::provider_status_is_usable(provider_status) {
        return (
            false,
            false,
            Some(
                crate::provider_usability::provider_status_unusable_reason(provider_status)
                    .unwrap_or_else(|| "provider not ready for use".to_string()),
            ),
        );
    }

    if !matches!(install_target, InstallTarget::Container) {
        return (true, false, None);
    }

    match provider_probe::provider_probe_env_for_workspace_runtime(state, workspace, provider_id)
        .await
    {
        Ok(_) => (true, false, None),
        Err(err) => {
            let probe_error = logs::redact_sensitive(&err);
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            (false, auth_required.unwrap_or(false), Some(probe_error))
        }
    }
}
