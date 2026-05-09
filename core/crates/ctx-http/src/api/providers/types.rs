use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use ctx_harness_sources as harness_sources;
use ctx_harness_sources::{HarnessApiShape, HarnessSourceKind};
use ctx_provider_accounts as provider_accounts;
use ctx_provider_auth_import as provider_auth_import;
use ctx_provider_runtime::provider_usage;
use ctx_providers::adapters::ProviderStatus;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct InstallTargetQuery {
    pub(super) target: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderUsageQuery {
    pub(super) refresh: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::CodexAccountEntry>,
    pub(super) logins: Vec<provider_accounts::CodexLoginStatus>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::ClaudeAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GeminiAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::GeminiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct QwenAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::QwenAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct KimiAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::KimiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MistralAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::MistralAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CopilotAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::CopilotAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CursorAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::CursorAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProvidersBootstrapResponse {
    pub(super) providers: Vec<ProviderStatus>,
    pub(super) provider_options: HashMap<String, serde_json::Value>,
    pub(super) provider_harness_config:
        HashMap<String, harness_sources::HarnessProviderSourceConfig>,
    pub(super) codex_accounts: CodexAccountsResponse,
    pub(super) claude_accounts: ClaudeAccountsResponse,
    pub(super) gemini_accounts: GeminiAccountsResponse,
    pub(super) qwen_accounts: QwenAccountsResponse,
    pub(super) kimi_accounts: KimiAccountsResponse,
    pub(super) mistral_accounts: MistralAccountsResponse,
    pub(super) copilot_accounts: CopilotAccountsResponse,
    pub(super) cursor_accounts: CursorAccountsResponse,
    pub(super) amp_accounts: AmpAccountsResponse,
}

#[derive(Debug, Serialize)]
pub(crate) struct AmpAccountsResponse {
    pub(super) active_account_id: Option<String>,
    pub(super) accounts: Vec<provider_accounts::AmpAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountUsageEntry {
    pub(super) account_id: Option<String>,
    pub(super) label: String,
    pub(super) email: Option<String>,
    pub(super) plan_type: Option<String>,
    pub(super) last_used_at: Option<DateTime<Utc>>,
    pub(super) usage: provider_usage::ProviderUsageSnapshot,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountsUsageResponse {
    pub(super) entries: Vec<CodexAccountUsageEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeAccountUpsertReq {
    pub(super) label: Option<String>,
    #[serde(alias = "auth_token")]
    pub(super) setup_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeminiAccountUpsertReq {
    pub(super) label: Option<String>,
    pub(super) oauth_creds_json: String,
    #[serde(default)]
    pub(super) google_accounts_json: Option<String>,
    #[serde(default)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QwenAccountUpsertReq {
    pub(super) label: Option<String>,
    pub(super) oauth_creds_json: String,
    #[serde(default)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AmpAccountUpsertReq {
    pub(super) label: Option<String>,
    #[serde(default)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MistralAccountUpsertReq {
    pub(super) label: Option<String>,
    #[serde(default)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeminiActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QwenActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KimiAccountUpsertReq {
    pub(super) label: Option<String>,
    #[serde(default)]
    pub(super) provider: Option<String>,
    pub(super) credentials_json: String,
    #[serde(default)]
    pub(super) config_toml: Option<String>,
    #[serde(default)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KimiActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MistralActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopilotAccountUpsertReq {
    pub(super) label: Option<String>,
    pub(super) token: String,
    #[serde(default)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopilotActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CursorAccountUpsertReq {
    pub(super) label: Option<String>,
    pub(super) token: String,
    #[serde(default)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CursorActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AmpActiveAccountReq {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProviderAuthImportCandidatesResponse {
    pub(super) candidates: Vec<provider_auth_import::ProviderAuthImportCandidate>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProviderAuthImportProfilesResponse {
    pub(super) profiles: Vec<provider_auth_import::ProviderImportedAuthProfile>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderAuthImportReq {
    pub(super) candidate_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProviderAuthImportResponse {
    pub(super) results: Vec<provider_auth_import::ProviderAuthImportResult>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexHostImportReq {
    pub(super) label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SelectHarnessSourceReq {
    pub(super) source_kind: HarnessSourceKind,
    #[serde(default)]
    pub(super) endpoint_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpsertHarnessEndpointReq {
    #[serde(default)]
    pub(super) endpoint_id: Option<String>,
    pub(super) name: String,
    #[serde(default)]
    pub(super) base_url: Option<String>,
    #[serde(default)]
    pub(super) api_shape: Option<HarnessApiShape>,
    #[serde(default)]
    pub(super) auth_type: Option<String>,
    #[serde(default)]
    pub(super) model_override: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) service_account_json: Option<String>,
    #[serde(default)]
    pub(super) project_id: Option<String>,
    #[serde(default)]
    pub(super) location: Option<String>,
    #[serde(default)]
    pub(super) manual_model_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetEndpointManualModelsReq {
    #[serde(default)]
    pub(super) model_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MatrixRefreshResponse {
    pub(super) provider_count: usize,
    pub(super) generated_at: Option<String>,
    pub(super) source: String,
    pub(super) degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DevRestartProvidersReq {
    pub(super) mode: String,
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DevRestartProvidersResp {
    pub(super) mode: String,
    pub(super) results: Vec<DevRestartProvidersResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DevRestartProvidersResult {
    pub(super) provider_id: String,
    pub(super) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) message: Option<String>,
}
