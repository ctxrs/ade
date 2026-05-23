use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;

use super::super::{ProviderAccountLoginMutation, ProviderAccountMutationError};
use crate::daemon::{providers::restarts, DaemonState};

pub async fn upsert_amp_account_for_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::upsert_amp_account(&state.core.data_root, label, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        restarts::restart_amp_providers_for_auth_change(state, "amp auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub async fn upsert_mistral_account_for_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::upsert_mistral_account(&state.core.data_root, label, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub async fn add_gemini_account_for_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_gemini_account(
        &state.core.data_root,
        label,
        oauth_creds_json,
        google_accounts_json,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub async fn add_qwen_account_for_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry =
        provider_accounts::add_qwen_account(&state.core.data_root, label, oauth_creds_json, email)
            .await
            .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}
