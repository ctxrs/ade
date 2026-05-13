use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;

use super::{ProviderAccountLoginMutation, ProviderAccountMutationError};
use crate::daemon::{providers::restarts, AppState};

pub(crate) async fn load_amp_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
    provider_accounts::load_amp_registry(&state.core.data_root).await
}

pub(crate) async fn ensure_amp_account_registry_from_runtime_auth(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
    provider_accounts::ensure_amp_registry_from_runtime_auth(&state.core.data_root).await
}

pub(crate) async fn load_claude_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::ClaudeAccountRegistry> {
    provider_accounts::load_claude_registry(&state.core.data_root).await
}

pub(crate) async fn load_copilot_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::CopilotAccountRegistry> {
    provider_accounts::load_copilot_registry(&state.core.data_root).await
}

pub(crate) async fn load_cursor_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::CursorAccountRegistry> {
    provider_accounts::load_cursor_registry(&state.core.data_root).await
}

pub(crate) async fn load_gemini_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::GeminiAccountRegistry> {
    provider_accounts::load_gemini_registry(&state.core.data_root).await
}

pub(crate) async fn load_kimi_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::KimiAccountRegistry> {
    provider_accounts::load_kimi_registry(&state.core.data_root).await
}

pub(crate) async fn load_mistral_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::MistralAccountRegistry> {
    provider_accounts::load_mistral_registry(&state.core.data_root).await
}

pub(crate) async fn load_qwen_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::QwenAccountRegistry> {
    provider_accounts::load_qwen_registry(&state.core.data_root).await
}

pub(crate) async fn upsert_amp_account(
    state: &Arc<AppState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    upsert_amp_account_for_login(state, label, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn upsert_amp_account_for_login(
    state: &Arc<AppState>,
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

pub(crate) async fn set_active_amp_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_amp_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_amp_providers_for_auth_change(state, "amp auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_amp_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_amp_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_amp_providers_for_auth_change(state, "amp auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_claude_account(
    state: &Arc<AppState>,
    label: Option<String>,
    setup_token: String,
) -> Result<(), ProviderAccountMutationError> {
    add_claude_account_for_login(state, label, setup_token)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn add_claude_account_for_login(
    state: &Arc<AppState>,
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

pub(crate) async fn set_active_claude_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_claude_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_claude_providers_for_auth_change(state, "claude auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_claude_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_claude_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_claude_providers_for_auth_change(state, "claude auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn upsert_mistral_account(
    state: &Arc<AppState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    upsert_mistral_account_for_login(state, label, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn upsert_mistral_account_for_login(
    state: &Arc<AppState>,
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

pub(crate) async fn set_active_mistral_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_mistral_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_mistral_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_mistral_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

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

pub(crate) async fn add_cursor_account(
    state: &Arc<AppState>,
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

pub(crate) async fn add_cursor_oauth_account_for_login(
    state: &Arc<AppState>,
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

pub(crate) async fn set_active_cursor_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_cursor_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_cursor_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_cursor_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_gemini_account(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    add_gemini_account_for_login(state, label, oauth_creds_json, google_accounts_json, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn add_gemini_account_for_login(
    state: &Arc<AppState>,
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

pub(crate) async fn set_active_gemini_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_gemini_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_gemini_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_gemini_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_kimi_account(
    state: &Arc<AppState>,
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

pub(crate) async fn add_kimi_oauth_account_for_login(
    state: &Arc<AppState>,
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

pub(crate) async fn set_active_kimi_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_kimi_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_kimi_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_kimi_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_qwen_account(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    add_qwen_account_for_login(state, label, oauth_creds_json, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn add_qwen_account_for_login(
    state: &Arc<AppState>,
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

pub(crate) async fn set_active_qwen_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_qwen_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_qwen_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_qwen_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}
