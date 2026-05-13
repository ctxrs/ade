use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;

use super::super::ProviderAccountMutationError;
use crate::daemon::{providers::restarts, AppState};

pub(crate) async fn add_copilot_account(
    state: &Arc<AppState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::add_copilot_account(&state.core.data_root, label, token, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_copilot_providers_for_auth_change(state, "copilot auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn set_active_copilot_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_copilot_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_copilot_providers_for_auth_change(state, "copilot auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_copilot_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_copilot_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_copilot_providers_for_auth_change(state, "copilot auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}
