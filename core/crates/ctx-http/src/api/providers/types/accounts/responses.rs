use chrono::{DateTime, Utc};
use serde::Serialize;

use ctx_provider_accounts as provider_accounts;
use ctx_provider_runtime::provider_usage;

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::CodexAccountEntry>,
    pub(in crate::api::providers) logins: Vec<provider_accounts::CodexLoginStatus>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::ClaudeAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GeminiAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::GeminiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct QwenAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::QwenAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct KimiAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::KimiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MistralAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::MistralAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CopilotAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::CopilotAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CursorAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::CursorAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AmpAccountsResponse {
    pub(in crate::api::providers) active_account_id: Option<String>,
    pub(in crate::api::providers) accounts: Vec<provider_accounts::AmpAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountUsageEntry {
    pub(in crate::api::providers) account_id: Option<String>,
    pub(in crate::api::providers) label: String,
    pub(in crate::api::providers) email: Option<String>,
    pub(in crate::api::providers) plan_type: Option<String>,
    pub(in crate::api::providers) last_used_at: Option<DateTime<Utc>>,
    pub(in crate::api::providers) usage: provider_usage::ProviderUsageSnapshot,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountsUsageResponse {
    pub(in crate::api::providers) entries: Vec<CodexAccountUsageEntry>,
}
