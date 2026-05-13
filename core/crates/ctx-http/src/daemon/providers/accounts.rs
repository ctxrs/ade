use std::{fmt, sync::Arc};

use ctx_provider_accounts as provider_accounts;

use crate::daemon::AppState;

pub(crate) struct CodexAccountsSnapshot {
    pub(crate) active_account_id: Option<String>,
    pub(crate) accounts: Vec<provider_accounts::CodexAccountEntry>,
    pub(crate) logins: Vec<provider_accounts::CodexLoginStatus>,
}

#[derive(Debug)]
pub(crate) enum ProviderAccountMutationError {
    BadRequest(anyhow::Error),
    Delete(anyhow::Error),
    Internal(anyhow::Error),
}

impl ProviderAccountMutationError {
    pub(crate) fn auth_login_error_message(&self) -> String {
        match self {
            Self::Internal(err) => format!("auth saved but provider restart failed: {err:#}"),
            Self::BadRequest(err) | Self::Delete(err) => err.to_string(),
        }
    }
}

impl fmt::Display for ProviderAccountMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(err) | Self::Delete(err) | Self::Internal(err) => {
                write!(f, "{err:#}")
            }
        }
    }
}

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

pub(crate) async fn load_codex_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::CodexAccountRegistry> {
    provider_accounts::load_codex_registry(&state.core.data_root).await
}

pub(crate) async fn load_codex_accounts_snapshot(
    state: &Arc<AppState>,
) -> anyhow::Result<CodexAccountsSnapshot> {
    let registry = load_codex_account_registry(state).await?;
    let logins = state
        .providers
        .with_codex_login_sessions(|map| map.values().cloned().collect::<Vec<_>>())
        .await;
    Ok(CodexAccountsSnapshot {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    })
}

pub(crate) async fn probe_host_codex_auth_candidate() -> provider_accounts::CodexHostImportProbe {
    provider_accounts::probe_host_codex_auth_candidate().await
}

pub(crate) async fn import_host_codex_auth(
    state: &Arc<AppState>,
    label: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::import_host_codex_auth_to_secret_store(&state.core.data_root, label)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn set_active_codex_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
    provider_accounts::set_active_codex_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    load_codex_accounts_snapshot(state)
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_codex_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
    let registry = provider_accounts::remove_codex_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    let logins = state
        .providers
        .with_codex_login_sessions(|map| {
            map.remove(account_id);
            map.values().cloned().collect::<Vec<_>>()
        })
        .await;
    Ok(CodexAccountsSnapshot {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    })
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

pub(crate) async fn add_gemini_account(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    add_gemini_account_for_login(state, label, oauth_creds_json, google_accounts_json, email)
        .await
        .map(|_| ())
}

pub(crate) async fn add_gemini_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<Option<String>, ProviderAccountMutationError> {
    let registry = provider_accounts::add_gemini_account(
        &state.core.data_root,
        label,
        oauth_creds_json,
        google_accounts_json,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    Ok(registry.active_account_id)
}

pub(crate) async fn set_active_gemini_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_gemini_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
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
    super::restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
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
    super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    Ok(())
}

pub(crate) async fn add_kimi_oauth_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    credentials_json: String,
    email: Option<String>,
) -> Result<Option<String>, ProviderAccountMutationError> {
    let registry = provider_accounts::add_kimi_oauth_account(
        &state.core.data_root,
        label,
        credentials_json,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    Ok(registry.active_account_id)
}

pub(crate) async fn set_active_kimi_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_kimi_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
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
    super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
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
        .map(|_| ())
}

pub(crate) async fn add_qwen_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<Option<String>, ProviderAccountMutationError> {
    let registry =
        provider_accounts::add_qwen_account(&state.core.data_root, label, oauth_creds_json, email)
            .await
            .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    Ok(registry.active_account_id)
}

pub(crate) async fn set_active_qwen_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_qwen_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
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
    super::restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}
