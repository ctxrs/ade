use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;
use serde::Serialize;

use super::{CodexAccountsSnapshot, ProviderAccountMutationError};
use crate::daemon::DaemonState;

#[derive(Debug, Serialize)]
pub struct CodexAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CodexAccountEntry>,
    logins: Vec<provider_accounts::CodexLoginStatus>,
}

#[derive(Debug, Serialize)]
pub struct ClaudeAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::ClaudeAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct GeminiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::GeminiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct QwenAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::QwenAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct KimiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::KimiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct MistralAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::MistralAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct CopilotAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CopilotAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct CursorAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CursorAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct AmpAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::AmpAccountEntry>,
}

impl CodexAccountsResponse {
    pub(in crate::daemon::providers) fn from_snapshot(snapshot: CodexAccountsSnapshot) -> Self {
        Self {
            active_account_id: snapshot.active_account_id,
            accounts: snapshot.accounts,
            logins: snapshot.logins,
        }
    }
}

macro_rules! account_response_from_registry {
    ($response:ty, $registry:ty) => {
        impl $response {
            pub(in crate::daemon::providers) fn from_registry(registry: $registry) -> Self {
                Self {
                    active_account_id: registry.active_account_id,
                    accounts: registry.accounts,
                }
            }
        }
    };
}

account_response_from_registry!(
    ClaudeAccountsResponse,
    provider_accounts::ClaudeAccountRegistry
);
account_response_from_registry!(
    GeminiAccountsResponse,
    provider_accounts::GeminiAccountRegistry
);
account_response_from_registry!(QwenAccountsResponse, provider_accounts::QwenAccountRegistry);
account_response_from_registry!(KimiAccountsResponse, provider_accounts::KimiAccountRegistry);
account_response_from_registry!(
    MistralAccountsResponse,
    provider_accounts::MistralAccountRegistry
);
account_response_from_registry!(
    CopilotAccountsResponse,
    provider_accounts::CopilotAccountRegistry
);
account_response_from_registry!(
    CursorAccountsResponse,
    provider_accounts::CursorAccountRegistry
);
account_response_from_registry!(AmpAccountsResponse, provider_accounts::AmpAccountRegistry);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ProviderAccountRouteErrorKind {
    BadRequest,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProviderAccountRouteError {
    kind: ProviderAccountRouteErrorKind,
    message: String,
}

impl ProviderAccountRouteError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAccountRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAccountRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAccountRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ProviderAccountRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

fn internal_error(error: impl ToString) -> ProviderAccountRouteError {
    ProviderAccountRouteError::internal(error.to_string())
}

fn unknown_account_error() -> ProviderAccountRouteError {
    ProviderAccountRouteError::not_found("unknown account")
}

fn provider_account_delete_error(err: anyhow::Error) -> ProviderAccountRouteError {
    let error = err.to_string();
    if error.contains("unknown account") {
        ProviderAccountRouteError::not_found(error)
    } else {
        ProviderAccountRouteError::internal(error)
    }
}

fn provider_account_mutation_error(err: ProviderAccountMutationError) -> ProviderAccountRouteError {
    match err {
        ProviderAccountMutationError::BadRequest(err) => {
            ProviderAccountRouteError::bad_request(err.to_string())
        }
        ProviderAccountMutationError::Delete(err) => provider_account_delete_error(err),
        ProviderAccountMutationError::Internal(err) => internal_error(err),
    }
}

fn codex_set_active_error(err: ProviderAccountMutationError) -> ProviderAccountRouteError {
    match err {
        ProviderAccountMutationError::BadRequest(err) => {
            let msg = err.to_string();
            if msg.contains("api_shape=openai_responses")
                || msg.contains("auth_type=bearer")
                || msg.contains("unknown account")
            {
                ProviderAccountRouteError::bad_request(msg)
            } else {
                ProviderAccountRouteError::internal(msg)
            }
        }
        ProviderAccountMutationError::Delete(err) => provider_account_delete_error(err),
        ProviderAccountMutationError::Internal(err) => internal_error(err),
    }
}

fn ensure_known_account<T>(
    account_id: &Option<String>,
    accounts: &[T],
    id: impl Fn(&T) -> &str,
) -> Result<(), ProviderAccountRouteError> {
    let Some(account_id) = account_id.as_ref() else {
        return Ok(());
    };
    if accounts.iter().any(|account| id(account) == account_id) {
        return Ok(());
    }

    Err(unknown_account_error())
}

pub async fn codex_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    super::load_codex_accounts_snapshot(state)
        .await
        .map(CodexAccountsResponse::from_snapshot)
        .map_err(internal_error)
}

pub async fn import_host_codex_auth_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    super::import_host_codex_auth(state, label)
        .await
        .map_err(provider_account_mutation_error)?;
    codex_accounts_response(state).await
}

pub async fn set_active_codex_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_codex_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_codex_account(state, account_id)
        .await
        .map(CodexAccountsResponse::from_snapshot)
        .map_err(codex_set_active_error)
}

pub async fn delete_codex_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
    super::remove_codex_account(state, account_id)
        .await
        .map(CodexAccountsResponse::from_snapshot)
        .map_err(provider_account_mutation_error)
}

pub async fn amp_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    super::ensure_amp_account_registry_from_runtime_auth(state)
        .await
        .map(AmpAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn upsert_amp_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    super::upsert_amp_account(state, label, email)
        .await
        .map_err(provider_account_mutation_error)?;
    amp_accounts_response(state).await
}

pub async fn set_active_amp_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_amp_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_amp_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    amp_accounts_response(state).await
}

pub async fn delete_amp_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
    super::remove_amp_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    amp_accounts_response(state).await
}

pub async fn claude_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    super::load_claude_account_registry(state)
        .await
        .map(ClaudeAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn add_claude_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    setup_token: String,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    super::add_claude_account(state, label, setup_token)
        .await
        .map_err(provider_account_mutation_error)?;
    claude_accounts_response(state).await
}

pub async fn set_active_claude_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_claude_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_claude_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    claude_accounts_response(state).await
}

pub async fn delete_claude_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
    super::remove_claude_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    claude_accounts_response(state).await
}

pub async fn copilot_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    super::load_copilot_account_registry(state)
        .await
        .map(CopilotAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn add_copilot_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    super::add_copilot_account(state, label, token, email)
        .await
        .map_err(provider_account_mutation_error)?;
    copilot_accounts_response(state).await
}

pub async fn set_active_copilot_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_copilot_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_copilot_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    copilot_accounts_response(state).await
}

pub async fn delete_copilot_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
    super::remove_copilot_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    copilot_accounts_response(state).await
}

pub async fn cursor_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    super::load_cursor_account_registry(state)
        .await
        .map(CursorAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn add_cursor_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    super::add_cursor_account(state, label, token, email)
        .await
        .map_err(provider_account_mutation_error)?;
    cursor_accounts_response(state).await
}

pub async fn set_active_cursor_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_cursor_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_cursor_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    cursor_accounts_response(state).await
}

pub async fn delete_cursor_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
    super::remove_cursor_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    cursor_accounts_response(state).await
}

pub async fn gemini_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    super::load_gemini_account_registry(state)
        .await
        .map(GeminiAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn add_gemini_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    super::add_gemini_account(state, label, oauth_creds_json, google_accounts_json, email)
        .await
        .map_err(provider_account_mutation_error)?;
    gemini_accounts_response(state).await
}

pub async fn set_active_gemini_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_gemini_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_gemini_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    gemini_accounts_response(state).await
}

pub async fn delete_gemini_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
    super::remove_gemini_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    gemini_accounts_response(state).await
}

pub async fn kimi_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    super::load_kimi_account_registry(state)
        .await
        .map(KimiAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn add_kimi_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    provider: Option<String>,
    credentials_json: String,
    config_toml: Option<String>,
    email: Option<String>,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    super::add_kimi_account(state, label, provider, credentials_json, config_toml, email)
        .await
        .map_err(provider_account_mutation_error)?;
    kimi_accounts_response(state).await
}

pub async fn set_active_kimi_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_kimi_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_kimi_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    kimi_accounts_response(state).await
}

pub async fn delete_kimi_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
    super::remove_kimi_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    kimi_accounts_response(state).await
}

pub async fn mistral_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    super::load_mistral_account_registry(state)
        .await
        .map(MistralAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn upsert_mistral_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    super::upsert_mistral_account(state, label, email)
        .await
        .map_err(provider_account_mutation_error)?;
    mistral_accounts_response(state).await
}

pub async fn set_active_mistral_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_mistral_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_mistral_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    mistral_accounts_response(state).await
}

pub async fn delete_mistral_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
    super::remove_mistral_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    mistral_accounts_response(state).await
}

pub async fn qwen_accounts_response(
    state: &Arc<DaemonState>,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    super::load_qwen_account_registry(state)
        .await
        .map(QwenAccountsResponse::from_registry)
        .map_err(internal_error)
}

pub async fn add_qwen_account_response(
    state: &Arc<DaemonState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    super::add_qwen_account(state, label, oauth_creds_json, email)
        .await
        .map_err(provider_account_mutation_error)?;
    qwen_accounts_response(state).await
}

pub async fn set_active_qwen_account_response(
    state: &Arc<DaemonState>,
    account_id: Option<String>,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    if account_id.is_some() {
        let registry = super::load_qwen_account_registry(state)
            .await
            .map_err(internal_error)?;
        ensure_known_account(&account_id, &registry.accounts, |account| &account.id)?;
    }
    super::set_active_qwen_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    qwen_accounts_response(state).await
}

pub async fn delete_qwen_account_response(
    state: &Arc<DaemonState>,
    account_id: &str,
) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
    super::remove_qwen_account(state, account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    qwen_accounts_response(state).await
}
