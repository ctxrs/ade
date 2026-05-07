use std::collections::{HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::mpsc;
use url::Url;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::installer;
use crate::logs;
use ctx_core::ids::WorkspaceId;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_harness_sources as harness_sources;
use ctx_harness_sources::{HarnessApiShape, HarnessEndpointUpsert, HarnessSourceKind};
use ctx_provider_accounts as provider_accounts;
use ctx_provider_auth_import as provider_auth_import;
#[cfg(test)]
use ctx_provider_install::install_state::InstallId;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::provider_usage;
use ctx_providers::adapters::{ProviderRestartMode, ProviderStatus};

mod accounts;
mod bootstrap;
#[cfg(test)]
mod codex_auth_tests;
mod cursor_login;
mod harness_config;
mod imports;
mod install;
mod login;
mod probe;
mod restarts;
mod status;

pub(super) use accounts::{
    delete_amp_account, delete_claude_account, delete_codex_account, delete_copilot_account,
    delete_cursor_account, delete_gemini_account, delete_kimi_account, delete_mistral_account,
    delete_qwen_account, get_codex_accounts_usage, import_host_codex_auth, list_amp_accounts,
    list_claude_accounts, list_codex_accounts, list_copilot_accounts, list_cursor_accounts,
    list_gemini_accounts, list_kimi_accounts, list_mistral_accounts, list_qwen_accounts,
    probe_host_codex_import, set_amp_active_account, set_claude_active_account,
    set_codex_active_account, set_copilot_active_account, set_cursor_active_account,
    set_gemini_active_account, set_kimi_active_account, set_mistral_active_account,
    set_qwen_active_account, upsert_amp_account, upsert_claude_account, upsert_copilot_account,
    upsert_cursor_account, upsert_gemini_account, upsert_kimi_account, upsert_mistral_account,
    upsert_qwen_account,
};
pub(super) use bootstrap::get_workspace_providers_bootstrap;
pub(super) use cursor_login::{get_cursor_login, start_cursor_login};
pub(super) use harness_config::{
    delete_provider_harness_endpoint, get_provider_harness_config,
    refresh_provider_harness_endpoint_models, select_provider_harness_source,
    set_provider_harness_endpoint_manual_models, upsert_provider_harness_endpoint,
};
pub(super) use imports::{
    import_provider_auth_candidates, list_provider_auth_import_candidates,
    list_provider_auth_import_profiles,
};
pub(super) use install::{dev_restart_providers, refresh_provider_matrix};
pub(super) use login::{
    complete_codex_login, get_amp_login, get_claude_login, get_codex_login, get_gemini_login,
    get_kimi_login, get_mistral_login, get_qwen_login, start_amp_login, start_claude_login,
    start_codex_login, start_gemini_login, start_kimi_login, start_mistral_login, start_qwen_login,
};
pub(super) use status::{get_provider, get_provider_usage, list_providers};
pub(crate) use status::{
    install_target_for_workspace, mark_provider_status_with_managed_config_error,
    provider_status_for_target, providers_statuses_response,
};

#[cfg(test)]
use imports::import_result_requires_provider_restart;
#[cfg(test)]
use login::{
    auth_url_looks_complete, expected_callback_from_auth_url, extract_auth_url,
    extract_auth_url_from_value, normalize_claude_login_line, read_trailing_claude_login_lines,
    resolve_claude_login_runtime_from_config, validate_callback_url,
};
#[cfg(test)]
use probe::*;
#[cfg(test)]
use restarts::*;

pub(super) fn canonicalize_provider_id(provider_id: &str) -> String {
    provider_id.to_string()
}

pub(super) fn project_provider_id_for_response(
    requested_provider_id: &str,
    canonical_provider_id_value: &str,
) -> String {
    let _ = requested_provider_id;
    canonical_provider_id_value.to_string()
}

pub(super) fn project_harness_config_for_response(
    requested_provider_id: &str,
    config: &mut harness_sources::HarnessProviderSourceConfig,
) {
    let projected_provider_id =
        project_provider_id_for_response(requested_provider_id, &config.provider_id);
    if projected_provider_id == config.provider_id {
        return;
    }
    config.provider_id = projected_provider_id.clone();
    for endpoint in &mut config.endpoints {
        endpoint.provider_id = projected_provider_id.clone();
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct InstallTargetQuery {
    target: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProviderUsageQuery {
    refresh: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CodexAccountEntry>,
    logins: Vec<provider_accounts::CodexLoginStatus>,
}

#[derive(Debug, Serialize)]
pub(super) struct ClaudeAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::ClaudeAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct GeminiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::GeminiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct QwenAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::QwenAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct KimiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::KimiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct MistralAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::MistralAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CopilotAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CopilotAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CursorAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CursorAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProvidersBootstrapResponse {
    providers: Vec<ProviderStatus>,
    provider_options: HashMap<String, serde_json::Value>,
    provider_harness_config: HashMap<String, harness_sources::HarnessProviderSourceConfig>,
    codex_accounts: CodexAccountsResponse,
    claude_accounts: ClaudeAccountsResponse,
    gemini_accounts: GeminiAccountsResponse,
    qwen_accounts: QwenAccountsResponse,
    kimi_accounts: KimiAccountsResponse,
    mistral_accounts: MistralAccountsResponse,
    copilot_accounts: CopilotAccountsResponse,
    cursor_accounts: CursorAccountsResponse,
    amp_accounts: AmpAccountsResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct AmpAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::AmpAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexAccountUsageEntry {
    account_id: Option<String>,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
    last_used_at: Option<DateTime<Utc>>,
    usage: provider_usage::ProviderUsageSnapshot,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexAccountsUsageResponse {
    entries: Vec<CodexAccountUsageEntry>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeAccountUpsertReq {
    label: Option<String>,
    #[serde(alias = "auth_token")]
    setup_token: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GeminiAccountUpsertReq {
    label: Option<String>,
    oauth_creds_json: String,
    #[serde(default)]
    google_accounts_json: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QwenAccountUpsertReq {
    label: Option<String>,
    oauth_creds_json: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AmpAccountUpsertReq {
    label: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MistralAccountUpsertReq {
    label: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GeminiActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QwenActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KimiAccountUpsertReq {
    label: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    credentials_json: String,
    #[serde(default)]
    config_toml: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KimiActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MistralActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CopilotAccountUpsertReq {
    label: Option<String>,
    token: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CopilotActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CursorAccountUpsertReq {
    label: Option<String>,
    token: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CursorActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AmpActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthImportCandidatesResponse {
    candidates: Vec<provider_auth_import::ProviderAuthImportCandidate>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthImportProfilesResponse {
    profiles: Vec<provider_auth_import::ProviderImportedAuthProfile>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProviderAuthImportReq {
    candidate_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthImportResponse {
    results: Vec<provider_auth_import::ProviderAuthImportResult>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexHostImportReq {
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SelectHarnessSourceReq {
    source_kind: HarnessSourceKind,
    #[serde(default)]
    endpoint_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpsertHarnessEndpointReq {
    #[serde(default)]
    endpoint_id: Option<String>,
    name: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_shape: Option<HarnessApiShape>,
    #[serde(default)]
    auth_type: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    service_account_json: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    manual_model_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetEndpointManualModelsReq {
    #[serde(default)]
    model_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct MatrixRefreshResponse {
    provider_count: usize,
    generated_at: Option<String>,
    source: String,
    degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DevRestartProvidersReq {
    mode: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DevRestartProvidersResp {
    mode: String,
    results: Vec<DevRestartProvidersResult>,
}

#[derive(Debug, Serialize)]
pub(super) struct DevRestartProvidersResult {
    provider_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[cfg(test)]
mod tests;
