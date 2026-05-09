use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize)]
pub(crate) struct CodexActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(alias = "auth_token")]
    pub(in crate::api::providers) setup_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeminiAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) oauth_creds_json: String,
    #[serde(default)]
    pub(in crate::api::providers) google_accounts_json: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QwenAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) oauth_creds_json: String,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AmpAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MistralAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeminiActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QwenActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KimiAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) provider: Option<String>,
    pub(in crate::api::providers) credentials_json: String,
    #[serde(default)]
    pub(in crate::api::providers) config_toml: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KimiActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MistralActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopilotAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) token: String,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopilotActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CursorAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) token: String,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CursorActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AmpActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexHostImportReq {
    pub(in crate::api::providers) label: Option<String>,
}
