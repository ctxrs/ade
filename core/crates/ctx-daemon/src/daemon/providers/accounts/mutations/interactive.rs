use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;

use super::super::{ProviderAccountLoginMutation, ProviderAccountMutationError};
use crate::daemon::{providers::restarts, DaemonState};

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
