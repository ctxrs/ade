use std::sync::Arc;

use crate::daemon::{providers::accounts, DaemonState};
use ctx_provider_accounts::{
    AmpAccountsResponse, ClaudeAccountsResponse, CodexAccountsResponse, CopilotAccountsResponse,
    CursorAccountsResponse, GeminiAccountsResponse, KimiAccountsResponse, MistralAccountsResponse,
    ProviderAccountRouteError, QwenAccountsResponse,
};

use super::error::{
    codex_set_active_error, ensure_known_account, internal_error, provider_account_mutation_error,
};

pub(in crate::daemon::providers::accounts::routes) async fn codex_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    accounts::load_codex_accounts_snapshot(state)
        .await
        .map(codex_accounts_route_response)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn import_host_codex_auth_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    accounts::import_host_codex_auth(state, label)
        .await
        .map_err(provider_account_mutation_error)?;
    codex_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_codex_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_codex_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_codex_account(state, account_id)
        .await
        .map(codex_accounts_route_response)
        .map_err(codex_set_active_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_codex_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_codex_account(state, account_id)
        .await
        .map(codex_accounts_route_response)
        .map_err(provider_account_mutation_error)
}

fn codex_accounts_route_response(
    snapshot: accounts::CodexAccountsSnapshot,
) -> CodexAccountsResponse {
    CodexAccountsResponse::new(
        snapshot.active_account_id,
        snapshot.accounts,
        snapshot.logins,
    )
}

pub(in crate::daemon::providers::accounts::routes) async fn amp_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    accounts::ensure_amp_account_registry_from_runtime_auth(state)
        .await
        .map(AmpAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn upsert_amp_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    accounts::upsert_amp_account(state, label, email)
        .await
        .map_err(provider_account_mutation_error)?;
    amp_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_amp_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_amp_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_amp_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    amp_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_amp_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_amp_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    amp_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn claude_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    accounts::load_claude_account_registry(state)
        .await
        .map(ClaudeAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn add_claude_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    setup_token: String,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    accounts::add_claude_account(state, label, setup_token)
        .await
        .map_err(provider_account_mutation_error)?;
    claude_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_claude_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_claude_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_claude_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    claude_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_claude_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_claude_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    claude_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn copilot_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    accounts::load_copilot_account_registry(state)
        .await
        .map(CopilotAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn add_copilot_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    accounts::add_copilot_account(state, label, token, email)
        .await
        .map_err(provider_account_mutation_error)?;
    copilot_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_copilot_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_copilot_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_copilot_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    copilot_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_copilot_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_copilot_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    copilot_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn cursor_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    accounts::load_cursor_account_registry(state)
        .await
        .map(CursorAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn add_cursor_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    accounts::add_cursor_account(state, label, token, email)
        .await
        .map_err(provider_account_mutation_error)?;
    cursor_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_cursor_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_cursor_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_cursor_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    cursor_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_cursor_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_cursor_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    cursor_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn gemini_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    accounts::load_gemini_account_registry(state)
        .await
        .map(GeminiAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn add_gemini_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    accounts::add_gemini_account(state, label, oauth_creds_json, google_accounts_json, email)
        .await
        .map_err(provider_account_mutation_error)?;
    gemini_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_gemini_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_gemini_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_gemini_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    gemini_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_gemini_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_gemini_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    gemini_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn kimi_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    accounts::load_kimi_account_registry(state)
        .await
        .map(KimiAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn add_kimi_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    provider: Option<String>,
    credentials_json: String,
    config_toml: Option<String>,
    email: Option<String>,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    accounts::add_kimi_account(state, label, provider, credentials_json, config_toml, email)
        .await
        .map_err(provider_account_mutation_error)?;
    kimi_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_kimi_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_kimi_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_kimi_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    kimi_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_kimi_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_kimi_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    kimi_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn mistral_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    accounts::load_mistral_account_registry(state)
        .await
        .map(MistralAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn upsert_mistral_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    accounts::upsert_mistral_account(state, label, email)
        .await
        .map_err(provider_account_mutation_error)?;
    mistral_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_mistral_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_mistral_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_mistral_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    mistral_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_mistral_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_mistral_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    mistral_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn qwen_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    accounts::load_qwen_account_registry(state)
        .await
        .map(QwenAccountsResponse::from)
        .map_err(internal_error)
}

pub(in crate::daemon::providers::accounts::routes) async fn add_qwen_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    accounts::add_qwen_account(state, label, oauth_creds_json, email)
        .await
        .map_err(provider_account_mutation_error)?;
    qwen_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn set_active_qwen_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = accounts::load_qwen_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    accounts::set_active_qwen_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    qwen_accounts_response(state).await
}

pub(in crate::daemon::providers::accounts::routes) async fn delete_qwen_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    accounts::remove_qwen_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    qwen_accounts_response(state).await
}
