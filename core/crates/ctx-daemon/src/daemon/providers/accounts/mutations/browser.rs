use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;

use super::super::{ProviderAccountLoginMutation, ProviderAccountMutationError};
use crate::daemon::{providers::restarts, DaemonState};

pub async fn upsert_amp_account(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    upsert_amp_account_for_login(state, label, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

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

pub async fn set_active_amp_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_amp_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_amp_providers_for_auth_change(state, "amp auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn remove_amp_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_amp_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_amp_providers_for_auth_change(state, "amp auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn upsert_mistral_account(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    upsert_mistral_account_for_login(state, label, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
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

pub async fn set_active_mistral_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_mistral_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn remove_mistral_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_mistral_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn add_gemini_account(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    add_gemini_account_for_login(state, label, oauth_creds_json, google_accounts_json, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
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

pub async fn set_active_gemini_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_gemini_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn remove_gemini_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_gemini_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn add_qwen_account(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    add_qwen_account_for_login(state, label, oauth_creds_json, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
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

pub async fn set_active_qwen_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_qwen_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn remove_qwen_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_qwen_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}
