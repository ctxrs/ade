use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;

use super::super::{ProviderAccountLoginMutation, ProviderAccountMutationError};
use crate::daemon::{providers::restarts, DaemonState};

pub async fn add_claude_account(
    state: &Arc<DaemonState>,
    label: Option<String>,
    setup_token: String,
) -> Result<(), ProviderAccountMutationError> {
    add_claude_account_for_login(state, label, setup_token)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub async fn add_claude_account_for_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
    setup_token: String,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_claude_account(&state.core.data_root, label, setup_token)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        restarts::restart_claude_providers_for_auth_change(state, "claude auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub async fn set_active_claude_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_claude_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_claude_providers_for_auth_change(state, "claude auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn remove_claude_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_claude_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_claude_providers_for_auth_change(state, "claude auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn add_cursor_account(
    state: &Arc<DaemonState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::add_cursor_account(&state.core.data_root, label, token, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn add_cursor_oauth_account_for_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
    auth_token: String,
    refresh_token: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_cursor_oauth_account(
        &state.core.data_root,
        label,
        auth_token,
        refresh_token,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub async fn set_active_cursor_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_cursor_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn remove_cursor_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_cursor_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn add_kimi_account(
    state: &Arc<DaemonState>,
    label: Option<String>,
    provider: Option<String>,
    credentials_json: String,
    config_toml: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::add_kimi_account(
        &state.core.data_root,
        label,
        provider,
        credentials_json,
        config_toml,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    Ok(())
}

pub async fn add_kimi_oauth_account_for_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
    credentials_json: String,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_kimi_oauth_account(
        &state.core.data_root,
        label,
        credentials_json,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub async fn set_active_kimi_account(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_kimi_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub async fn remove_kimi_account(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_kimi_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}
