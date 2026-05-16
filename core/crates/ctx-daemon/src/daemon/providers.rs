use std::path::Path as StdPath;

use ctx_core::ids::WorkspaceId;
use ctx_harness_sources as harness_sources;
use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::provider_usage;
use ctx_provider_runtime::provider_workers::ProviderAdapterRestartResult;
use ctx_providers::adapters::{ProviderRestartMode, ProviderStatus};
use serde_json::Value;
use tokio::sync::broadcast;

use super::handle::ProvidersHandle;
mod accounts;
mod auth;
mod auth_check;
mod auth_import;
mod bootstrap;
mod browser_logins;
mod diagnostics;
mod harness_config;
mod installs;
mod inventory;
mod launch_config;
mod login_runtime;
mod login_sessions;
mod options;
mod options_cache;
mod restarts;
mod runtime_probe;
mod status;
mod usage;

pub use accounts::{
    add_claude_account, add_claude_account_for_login, add_copilot_account, add_cursor_account,
    add_cursor_oauth_account_for_login, add_gemini_account, add_gemini_account_for_login,
    add_kimi_account, add_kimi_oauth_account_for_login, add_qwen_account,
    add_qwen_account_for_login, ensure_amp_account_registry_from_runtime_auth,
    import_host_codex_auth, load_amp_account_registry, load_claude_account_registry,
    load_codex_account_registry, load_codex_accounts_snapshot, load_copilot_account_registry,
    load_cursor_account_registry, load_gemini_account_registry, load_kimi_account_registry,
    load_mistral_account_registry, load_qwen_account_registry, persist_successful_codex_login,
    prepare_codex_login_start, probe_host_codex_auth_candidate, remove_amp_account,
    remove_claude_account, remove_codex_account, remove_copilot_account, remove_cursor_account,
    remove_gemini_account, remove_kimi_account, remove_mistral_account, remove_qwen_account,
    set_active_amp_account, set_active_claude_account, set_active_codex_account,
    set_active_copilot_account, set_active_cursor_account, set_active_gemini_account,
    set_active_kimi_account, set_active_mistral_account, set_active_qwen_account,
    upsert_amp_account, upsert_amp_account_for_login, upsert_mistral_account,
    upsert_mistral_account_for_login, CodexAccountsSnapshot, PreparedCodexLoginStart,
    ProviderAccountLoginMutation, ProviderAccountMutationError,
};
pub use accounts::{
    AmpAccountsResponse, ClaudeAccountsResponse, CodexAccountsResponse, CopilotAccountsResponse,
    CursorAccountsResponse, GeminiAccountsResponse, KimiAccountsResponse, MistralAccountsResponse,
    ProviderAccountRouteError, ProviderAccountRouteErrorKind, QwenAccountsResponse,
};
pub use auth::{authenticate_provider_for_workspace_runtime, ProviderWorkspaceAuthenticationError};
pub use auth_check::{
    authenticate_provider_for_workspace, verify_provider_for_workspace, ProviderAuthCheckError,
    ProviderAuthCheckSnapshot,
};
pub use auth_import::provider_auth_import_result_requires_restart;
pub use auth_import::{
    import_provider_auth_candidates, list_provider_auth_import_candidates,
    list_provider_auth_import_profiles,
};
pub use bootstrap::{
    workspace_providers_bootstrap, ProvidersBootstrapError, ProvidersBootstrapErrorKind,
    ProvidersBootstrapResponse,
};
pub use browser_logins::{
    start_amp_browser_login, start_gemini_browser_login, start_mistral_browser_login,
    start_qwen_browser_login,
};
pub use diagnostics::provider_diagnostics_snapshot;
pub use harness_config::{
    delete_provider_harness_endpoint, get_provider_harness_config,
    mark_provider_endpoint_verification, refresh_provider_endpoint_model_catalog,
    refresh_provider_harness_endpoint_models, select_provider_harness_source,
    set_provider_harness_endpoint_manual_models, upsert_provider_harness_endpoint,
};
pub use installs::{
    cancel_provider_install, get_provider_install_info, list_provider_install_events,
    parse_provider_install_target, provider_install_event_sender, start_all_provider_installs,
    start_provider_install, StartProviderInstallError,
};
pub use inventory::refresh_provider_inventory;
pub use launch_config::{
    load_provider_launch_config_snapshot, ProviderLaunchConfigError, ProviderLaunchConfigSnapshot,
};
pub use login_runtime::{
    resolve_claude_login_runtime, resolve_cursor_login_runtime, ProviderLoginRuntimeCommand,
};
pub use login_runtime::{
    resolve_claude_login_runtime_from_config, resolve_cursor_login_runtime_from_config,
};
pub use login_sessions::{
    amp_login_status, claim_codex_login_callback, claude_login_status, codex_login_status,
    codex_login_statuses, cursor_login_status, finish_amp_login_session,
    finish_claude_login_session, finish_codex_login_session, finish_cursor_login_session,
    finish_gemini_login_session, finish_kimi_login_session, finish_mistral_login_session,
    finish_qwen_login_session, gemini_login_status, kimi_login_status, mistral_login_status,
    qwen_login_status, remove_codex_login_session, restore_codex_login_completion_token,
    set_amp_login_auth_url, set_amp_login_failed, set_amp_login_failed_if_no_error,
    set_amp_login_timeout_if_no_error, set_claude_login_auth_url, set_cursor_login_error,
    set_gemini_login_auth_url, set_gemini_login_failed, set_gemini_login_failed_if_no_error,
    set_gemini_login_timeout_if_no_error, set_kimi_login_failed, set_kimi_login_terminal_status,
    set_kimi_login_timeout_if_no_error, set_mistral_login_auth_url, set_mistral_login_failed,
    set_mistral_login_failed_if_no_error, set_mistral_login_timeout_if_no_error,
    set_qwen_login_auth_url, set_qwen_login_failed, set_qwen_login_failed_if_no_error,
    set_qwen_login_timeout_if_no_error, start_amp_login_session, start_claude_login_session,
    start_codex_login_session, start_cursor_login_session, start_gemini_login_session,
    start_kimi_login_session, start_mistral_login_session, start_qwen_login_session,
    update_cursor_login_auth_url, CodexLoginCallbackClaimError, StartedCodexLoginSession,
    StartedLoginSession,
};
pub use options::{
    effective_preferred_model_id_for_workspace, get_provider_options_response,
    EffectivePreferredModelError, ProviderOptionsResponseError,
};
pub use options_cache::{store_provider_verify_cache_value, ProviderOptionsCacheSnapshot};
pub use restarts::{
    restart_codex_providers_for_auth_change, restart_provider_for_auth_change,
    stop_codex_providers_for_auth_removal,
};
pub use runtime_probe::{
    probe_provider_auth_verification_runtime, probe_provider_options_env,
    probe_runtime_models_for_provider_options, probe_selected_endpoint_runtime_launch,
    provider_has_active_auth_for_workspace_runtime,
};
pub use status::{
    install_target_for_workspace, provider_status_response, providers_statuses_response,
    refresh_provider_statuses, ProviderStatusResponseError,
};
pub use usage::{load_codex_accounts_usage, load_provider_usage, CodexAccountUsageRecord};

impl ProvidersHandle {
    pub async fn provider_diagnostics_snapshot(&self) -> diagnostics::ProviderDiagnosticsSnapshot {
        provider_diagnostics_snapshot(&self.state).await
    }

    pub async fn can_create_loaded_session_for_provider(&self, provider_id: &str) -> bool {
        self.state
            .providers
            .can_create_loaded_session_for_provider(provider_id)
            .await
    }

    pub async fn refresh_provider_statuses(&self) -> anyhow::Result<()> {
        refresh_provider_statuses(self.state.as_ref()).await
    }

    pub async fn refresh_provider_inventory(
        &self,
    ) -> anyhow::Result<inventory::ProviderMatrixRefreshSummary> {
        refresh_provider_inventory(self.state.as_ref()).await
    }

    pub async fn restart_all_provider_adapters(
        &self,
        reason: &str,
        mode: ProviderRestartMode,
    ) -> Vec<ProviderAdapterRestartResult> {
        self.state
            .providers
            .restart_all_provider_adapters(reason, mode)
            .await
    }

    pub async fn restart_provider_for_auth_change(
        &self,
        provider_id: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        restart_provider_for_auth_change(&self.state, provider_id, reason).await
    }

    pub async fn install_target_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<InstallTarget> {
        install_target_for_workspace(&self.state, workspace_id).await
    }

    pub async fn providers_statuses_response(
        &self,
        target: InstallTarget,
        include_matrix_providers: bool,
    ) -> Vec<ProviderStatus> {
        providers_statuses_response(&self.state, target, include_matrix_providers).await
    }

    pub async fn provider_status_response(
        &self,
        provider_id: &str,
        target: InstallTarget,
    ) -> Result<ProviderStatus, ProviderStatusResponseError> {
        provider_status_response(&self.state, provider_id, target).await
    }

    pub async fn load_provider_usage(
        &self,
        provider_id: &str,
        refresh: bool,
    ) -> anyhow::Result<provider_usage::ProviderUsageSnapshot> {
        load_provider_usage(&self.state, provider_id, refresh).await
    }

    pub async fn load_codex_accounts_usage(
        &self,
        refresh: bool,
    ) -> anyhow::Result<Vec<CodexAccountUsageRecord>> {
        load_codex_accounts_usage(&self.state, refresh).await
    }

    pub async fn load_codex_accounts_snapshot(&self) -> anyhow::Result<CodexAccountsSnapshot> {
        load_codex_accounts_snapshot(&self.state).await
    }

    pub async fn codex_accounts_response(
        &self,
    ) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
        accounts::codex_accounts_response(&self.state).await
    }

    pub async fn load_codex_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::CodexAccountRegistry> {
        load_codex_account_registry(&self.state).await
    }

    pub async fn import_host_codex_auth(
        &self,
        label: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        import_host_codex_auth(&self.state, label).await
    }

    pub async fn import_host_codex_auth_response(
        &self,
        label: Option<String>,
    ) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
        accounts::import_host_codex_auth_response(&self.state, label).await
    }

    pub async fn set_active_codex_account(
        &self,
        account_id: Option<String>,
    ) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
        set_active_codex_account(&self.state, account_id).await
    }

    pub async fn set_active_codex_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_codex_account_response(&self.state, account_id).await
    }

    pub async fn remove_codex_account(
        &self,
        account_id: &str,
    ) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
        remove_codex_account(&self.state, account_id).await
    }

    pub async fn delete_codex_account_response(
        &self,
        account_id: &str,
    ) -> Result<CodexAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_codex_account_response(&self.state, account_id).await
    }

    pub async fn ensure_amp_account_registry_from_runtime_auth(
        &self,
    ) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
        ensure_amp_account_registry_from_runtime_auth(&self.state).await
    }

    pub async fn amp_accounts_response(
        &self,
    ) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
        accounts::amp_accounts_response(&self.state).await
    }

    pub async fn load_amp_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
        load_amp_account_registry(&self.state).await
    }

    pub async fn upsert_amp_account(
        &self,
        label: Option<String>,
        email: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        upsert_amp_account(&self.state, label, email).await
    }

    pub async fn upsert_amp_account_response(
        &self,
        label: Option<String>,
        email: Option<String>,
    ) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
        accounts::upsert_amp_account_response(&self.state, label, email).await
    }

    pub async fn set_active_amp_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_amp_account(&self.state, account_id).await
    }

    pub async fn set_active_amp_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_amp_account_response(&self.state, account_id).await
    }

    pub async fn remove_amp_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_amp_account(&self.state, account_id).await
    }

    pub async fn delete_amp_account_response(
        &self,
        account_id: &str,
    ) -> Result<AmpAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_amp_account_response(&self.state, account_id).await
    }

    pub async fn claude_accounts_response(
        &self,
    ) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
        accounts::claude_accounts_response(&self.state).await
    }

    pub async fn load_claude_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::ClaudeAccountRegistry> {
        load_claude_account_registry(&self.state).await
    }

    pub async fn add_claude_account(
        &self,
        label: Option<String>,
        setup_token: String,
    ) -> Result<(), ProviderAccountMutationError> {
        add_claude_account(&self.state, label, setup_token).await
    }

    pub async fn add_claude_account_response(
        &self,
        label: Option<String>,
        setup_token: String,
    ) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
        accounts::add_claude_account_response(&self.state, label, setup_token).await
    }

    pub async fn set_active_claude_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_claude_account(&self.state, account_id).await
    }

    pub async fn set_active_claude_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_claude_account_response(&self.state, account_id).await
    }

    pub async fn remove_claude_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_claude_account(&self.state, account_id).await
    }

    pub async fn delete_claude_account_response(
        &self,
        account_id: &str,
    ) -> Result<ClaudeAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_claude_account_response(&self.state, account_id).await
    }

    pub async fn copilot_accounts_response(
        &self,
    ) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
        accounts::copilot_accounts_response(&self.state).await
    }

    pub async fn load_copilot_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::CopilotAccountRegistry> {
        load_copilot_account_registry(&self.state).await
    }

    pub async fn add_copilot_account(
        &self,
        label: Option<String>,
        token: String,
        email: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        add_copilot_account(&self.state, label, token, email).await
    }

    pub async fn add_copilot_account_response(
        &self,
        label: Option<String>,
        token: String,
        email: Option<String>,
    ) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
        accounts::add_copilot_account_response(&self.state, label, token, email).await
    }

    pub async fn set_active_copilot_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_copilot_account(&self.state, account_id).await
    }

    pub async fn set_active_copilot_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_copilot_account_response(&self.state, account_id).await
    }

    pub async fn remove_copilot_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_copilot_account(&self.state, account_id).await
    }

    pub async fn delete_copilot_account_response(
        &self,
        account_id: &str,
    ) -> Result<CopilotAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_copilot_account_response(&self.state, account_id).await
    }

    pub async fn cursor_accounts_response(
        &self,
    ) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
        accounts::cursor_accounts_response(&self.state).await
    }

    pub async fn load_cursor_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::CursorAccountRegistry> {
        load_cursor_account_registry(&self.state).await
    }

    pub async fn add_cursor_account(
        &self,
        label: Option<String>,
        token: String,
        email: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        add_cursor_account(&self.state, label, token, email).await
    }

    pub async fn add_cursor_account_response(
        &self,
        label: Option<String>,
        token: String,
        email: Option<String>,
    ) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
        accounts::add_cursor_account_response(&self.state, label, token, email).await
    }

    pub async fn set_active_cursor_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_cursor_account(&self.state, account_id).await
    }

    pub async fn set_active_cursor_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_cursor_account_response(&self.state, account_id).await
    }

    pub async fn remove_cursor_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_cursor_account(&self.state, account_id).await
    }

    pub async fn delete_cursor_account_response(
        &self,
        account_id: &str,
    ) -> Result<CursorAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_cursor_account_response(&self.state, account_id).await
    }

    pub async fn gemini_accounts_response(
        &self,
    ) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
        accounts::gemini_accounts_response(&self.state).await
    }

    pub async fn load_gemini_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::GeminiAccountRegistry> {
        load_gemini_account_registry(&self.state).await
    }

    pub async fn add_gemini_account(
        &self,
        label: Option<String>,
        oauth_creds_json: String,
        google_accounts_json: Option<String>,
        email: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        add_gemini_account(
            &self.state,
            label,
            oauth_creds_json,
            google_accounts_json,
            email,
        )
        .await
    }

    pub async fn add_gemini_account_response(
        &self,
        label: Option<String>,
        oauth_creds_json: String,
        google_accounts_json: Option<String>,
        email: Option<String>,
    ) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
        accounts::add_gemini_account_response(
            &self.state,
            label,
            oauth_creds_json,
            google_accounts_json,
            email,
        )
        .await
    }

    pub async fn set_active_gemini_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_gemini_account(&self.state, account_id).await
    }

    pub async fn set_active_gemini_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_gemini_account_response(&self.state, account_id).await
    }

    pub async fn remove_gemini_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_gemini_account(&self.state, account_id).await
    }

    pub async fn delete_gemini_account_response(
        &self,
        account_id: &str,
    ) -> Result<GeminiAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_gemini_account_response(&self.state, account_id).await
    }

    pub async fn kimi_accounts_response(
        &self,
    ) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
        accounts::kimi_accounts_response(&self.state).await
    }

    pub async fn load_kimi_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::KimiAccountRegistry> {
        load_kimi_account_registry(&self.state).await
    }

    pub async fn add_kimi_account(
        &self,
        label: Option<String>,
        provider: Option<String>,
        credentials_json: String,
        config_toml: Option<String>,
        email: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        add_kimi_account(
            &self.state,
            label,
            provider,
            credentials_json,
            config_toml,
            email,
        )
        .await
    }

    pub async fn add_kimi_account_response(
        &self,
        label: Option<String>,
        provider: Option<String>,
        credentials_json: String,
        config_toml: Option<String>,
        email: Option<String>,
    ) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
        accounts::add_kimi_account_response(
            &self.state,
            label,
            provider,
            credentials_json,
            config_toml,
            email,
        )
        .await
    }

    pub async fn set_active_kimi_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_kimi_account(&self.state, account_id).await
    }

    pub async fn set_active_kimi_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_kimi_account_response(&self.state, account_id).await
    }

    pub async fn remove_kimi_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_kimi_account(&self.state, account_id).await
    }

    pub async fn delete_kimi_account_response(
        &self,
        account_id: &str,
    ) -> Result<KimiAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_kimi_account_response(&self.state, account_id).await
    }

    pub async fn mistral_accounts_response(
        &self,
    ) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
        accounts::mistral_accounts_response(&self.state).await
    }

    pub async fn load_mistral_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::MistralAccountRegistry> {
        load_mistral_account_registry(&self.state).await
    }

    pub async fn upsert_mistral_account(
        &self,
        label: Option<String>,
        email: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        upsert_mistral_account(&self.state, label, email).await
    }

    pub async fn upsert_mistral_account_response(
        &self,
        label: Option<String>,
        email: Option<String>,
    ) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
        accounts::upsert_mistral_account_response(&self.state, label, email).await
    }

    pub async fn set_active_mistral_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_mistral_account(&self.state, account_id).await
    }

    pub async fn set_active_mistral_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_mistral_account_response(&self.state, account_id).await
    }

    pub async fn remove_mistral_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_mistral_account(&self.state, account_id).await
    }

    pub async fn delete_mistral_account_response(
        &self,
        account_id: &str,
    ) -> Result<MistralAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_mistral_account_response(&self.state, account_id).await
    }

    pub async fn qwen_accounts_response(
        &self,
    ) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
        accounts::qwen_accounts_response(&self.state).await
    }

    pub async fn load_qwen_account_registry(
        &self,
    ) -> anyhow::Result<provider_accounts::QwenAccountRegistry> {
        load_qwen_account_registry(&self.state).await
    }

    pub async fn add_qwen_account(
        &self,
        label: Option<String>,
        oauth_creds_json: String,
        email: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        add_qwen_account(&self.state, label, oauth_creds_json, email).await
    }

    pub async fn add_qwen_account_response(
        &self,
        label: Option<String>,
        oauth_creds_json: String,
        email: Option<String>,
    ) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
        accounts::add_qwen_account_response(&self.state, label, oauth_creds_json, email).await
    }

    pub async fn set_active_qwen_account(
        &self,
        account_id: Option<String>,
    ) -> Result<(), ProviderAccountMutationError> {
        set_active_qwen_account(&self.state, account_id).await
    }

    pub async fn set_active_qwen_account_response(
        &self,
        account_id: Option<String>,
    ) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
        accounts::set_active_qwen_account_response(&self.state, account_id).await
    }

    pub async fn remove_qwen_account(
        &self,
        account_id: &str,
    ) -> Result<(), ProviderAccountMutationError> {
        remove_qwen_account(&self.state, account_id).await
    }

    pub async fn delete_qwen_account_response(
        &self,
        account_id: &str,
    ) -> Result<QwenAccountsResponse, ProviderAccountRouteError> {
        accounts::delete_qwen_account_response(&self.state, account_id).await
    }

    pub async fn workspace_providers_bootstrap(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<ProvidersBootstrapResponse, ProvidersBootstrapError> {
        workspace_providers_bootstrap(&self.state, workspace_id).await
    }

    pub async fn get_provider_options_response(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
    ) -> Result<Value, ProviderOptionsResponseError> {
        get_provider_options_response(&self.state, workspace_id, provider_id).await
    }

    pub async fn authenticate_provider_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
        method_id: Option<String>,
    ) -> Result<ProviderAuthCheckSnapshot, ProviderAuthCheckError> {
        authenticate_provider_for_workspace(&self.state, workspace_id, provider_id, method_id).await
    }

    pub async fn verify_provider_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
    ) -> Result<ProviderAuthCheckSnapshot, ProviderAuthCheckError> {
        verify_provider_for_workspace(&self.state, workspace_id, provider_id).await
    }

    pub async fn start_provider_install(
        &self,
        provider_id: &str,
        target: InstallTarget,
    ) -> Result<InstallId, StartProviderInstallError> {
        start_provider_install(&self.state, provider_id, target).await
    }

    pub async fn start_all_provider_installs(
        &self,
        target: InstallTarget,
    ) -> Result<Vec<(String, InstallId)>, StartProviderInstallError> {
        start_all_provider_installs(&self.state, target).await
    }

    pub async fn get_provider_install_info(&self, install_id: InstallId) -> Option<InstallInfo> {
        get_provider_install_info(&self.state, install_id).await
    }

    pub async fn cancel_provider_install(&self, install_id: InstallId) -> Option<InstallInfo> {
        cancel_provider_install(&self.state, install_id).await
    }

    pub async fn list_provider_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        list_provider_install_events(&self.state, install_id).await
    }

    pub async fn provider_install_event_sender(
        &self,
        install_id: InstallId,
    ) -> Option<broadcast::Sender<InstallProgressEvent>> {
        provider_install_event_sender(&self.state, install_id).await
    }

    pub async fn get_provider_harness_config(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
        get_provider_harness_config(&self.state, provider_id).await
    }

    pub async fn select_provider_harness_source(
        &self,
        provider_id: &str,
        source_kind: harness_sources::HarnessSourceKind,
        endpoint_id: Option<String>,
    ) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
        select_provider_harness_source(&self.state, provider_id, source_kind, endpoint_id).await
    }

    pub async fn upsert_provider_harness_endpoint(
        &self,
        provider_id: &str,
        endpoint: harness_sources::HarnessEndpointUpsert,
        manual_model_ids: Option<Vec<String>>,
    ) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
        upsert_provider_harness_endpoint(&self.state, provider_id, endpoint, manual_model_ids).await
    }

    pub async fn refresh_provider_harness_endpoint_models(
        &self,
        provider_id: &str,
        endpoint_id: &str,
    ) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
        refresh_provider_harness_endpoint_models(&self.state, provider_id, endpoint_id).await
    }

    pub async fn set_provider_harness_endpoint_manual_models(
        &self,
        provider_id: &str,
        endpoint_id: &str,
        model_ids: Vec<String>,
    ) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
        set_provider_harness_endpoint_manual_models(
            &self.state,
            provider_id,
            endpoint_id,
            model_ids,
        )
        .await
    }

    pub async fn delete_provider_harness_endpoint(
        &self,
        provider_id: &str,
        endpoint_id: &str,
    ) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
        delete_provider_harness_endpoint(&self.state, provider_id, endpoint_id).await
    }

    pub async fn list_provider_auth_import_profiles(
        &self,
    ) -> anyhow::Result<Vec<ctx_provider_auth_import::ProviderImportedAuthProfile>> {
        list_provider_auth_import_profiles(&self.state).await
    }

    pub async fn import_provider_auth_candidates(
        &self,
        candidate_ids: Vec<String>,
    ) -> anyhow::Result<Vec<ctx_provider_auth_import::ProviderAuthImportResult>> {
        import_provider_auth_candidates(&self.state, candidate_ids).await
    }

    pub fn data_root(&self) -> &StdPath {
        &self.state.core.data_root
    }

    pub async fn add_kimi_oauth_account_for_login(
        &self,
        label: Option<String>,
        credentials_json: String,
        email: Option<String>,
    ) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
        add_kimi_oauth_account_for_login(&self.state, label, credentials_json, email).await
    }

    pub async fn start_amp_browser_login(&self, label: Option<String>) -> StartedLoginSession {
        start_amp_browser_login(&self.state, label).await
    }

    pub async fn amp_login_status(
        &self,
        login_id: &str,
    ) -> Option<provider_accounts::AmpLoginStatus> {
        amp_login_status(&self.state, login_id).await
    }

    pub async fn start_gemini_browser_login(&self, label: Option<String>) -> StartedLoginSession {
        start_gemini_browser_login(&self.state, label).await
    }

    pub async fn gemini_login_status(
        &self,
        login_id: &str,
    ) -> Option<provider_accounts::GeminiLoginStatus> {
        gemini_login_status(&self.state, login_id).await
    }

    pub async fn start_qwen_browser_login(&self, label: Option<String>) -> StartedLoginSession {
        start_qwen_browser_login(&self.state, label).await
    }

    pub async fn qwen_login_status(
        &self,
        login_id: &str,
    ) -> Option<provider_accounts::QwenLoginStatus> {
        qwen_login_status(&self.state, login_id).await
    }

    pub async fn start_mistral_browser_login(&self, label: Option<String>) -> StartedLoginSession {
        start_mistral_browser_login(&self.state, label).await
    }

    pub async fn mistral_login_status(
        &self,
        login_id: &str,
    ) -> Option<provider_accounts::MistralLoginStatus> {
        mistral_login_status(&self.state, login_id).await
    }

    pub async fn start_kimi_login_session(
        &self,
        auth_url: Option<String>,
        device_code: Option<String>,
    ) -> StartedLoginSession {
        start_kimi_login_session(&self.state, auth_url, device_code).await
    }

    pub async fn kimi_login_status(
        &self,
        login_id: &str,
    ) -> Option<provider_accounts::KimiLoginStatus> {
        kimi_login_status(&self.state, login_id).await
    }

    pub async fn set_kimi_login_failed(&self, login_id: &str, error: String) {
        set_kimi_login_failed(&self.state, login_id, error).await;
    }

    pub async fn set_kimi_login_timeout_if_no_error(&self, login_id: &str, error: String) {
        set_kimi_login_timeout_if_no_error(&self.state, login_id, error).await;
    }

    pub async fn set_kimi_login_terminal_status(
        &self,
        login_id: &str,
        status: &'static str,
        error: String,
    ) {
        set_kimi_login_terminal_status(&self.state, login_id, status, error).await;
    }

    pub async fn finish_kimi_login_session(
        &self,
        login_id: &str,
        account_id: Option<String>,
        restart_error: Option<String>,
    ) {
        finish_kimi_login_session(&self.state, login_id, account_id, restart_error).await;
    }

    pub async fn prepare_codex_login_start(
        &self,
        label: Option<String>,
    ) -> anyhow::Result<PreparedCodexLoginStart> {
        prepare_codex_login_start(&self.state, label).await
    }

    pub async fn start_codex_login_session(
        &self,
        account_id: String,
        auth_url: String,
        expected_callback_url: Option<String>,
    ) -> StartedCodexLoginSession {
        start_codex_login_session(&self.state, account_id, auth_url, expected_callback_url).await
    }

    pub async fn codex_login_status(
        &self,
        account_id: &str,
    ) -> Option<provider_accounts::CodexLoginStatus> {
        codex_login_status(&self.state, account_id).await
    }

    pub async fn claim_codex_login_callback(
        &self,
        account_id: &str,
        completion_token: &str,
    ) -> Result<String, CodexLoginCallbackClaimError> {
        claim_codex_login_callback(&self.state, account_id, completion_token).await
    }

    pub async fn restore_codex_login_completion_token(
        &self,
        account_id: &str,
        completion_token: &str,
    ) {
        restore_codex_login_completion_token(&self.state, account_id, completion_token).await;
    }

    pub async fn persist_successful_codex_login(
        &self,
        account_id: &str,
        label: String,
        email: Option<String>,
        plan_type: Option<String>,
    ) -> anyhow::Result<()> {
        persist_successful_codex_login(&self.state, account_id, label, email, plan_type).await
    }

    pub async fn finish_codex_login_session(
        &self,
        account_id: &str,
        success: bool,
        error: Option<String>,
    ) {
        finish_codex_login_session(&self.state, account_id, success, error).await;
    }

    pub async fn start_claude_login_session(
        &self,
        auth_url: Option<String>,
    ) -> StartedLoginSession {
        start_claude_login_session(&self.state, auth_url).await
    }

    pub async fn claude_login_status(
        &self,
        login_id: &str,
    ) -> Option<provider_accounts::ClaudeLoginStatus> {
        claude_login_status(&self.state, login_id).await
    }

    pub async fn set_claude_login_auth_url(&self, login_id: &str, auth_url: String) {
        set_claude_login_auth_url(&self.state, login_id, auth_url).await;
    }

    pub async fn finish_claude_login_session(
        &self,
        login_id: &str,
        status: String,
        account_id: Option<String>,
        error: Option<String>,
        observed_auth_url: Option<String>,
    ) {
        finish_claude_login_session(
            &self.state,
            login_id,
            status,
            account_id,
            error,
            observed_auth_url,
        )
        .await;
    }

    pub async fn resolve_claude_login_runtime(
        &self,
    ) -> anyhow::Result<ProviderLoginRuntimeCommand> {
        resolve_claude_login_runtime(&self.state).await
    }

    pub async fn add_claude_account_for_login(
        &self,
        label: Option<String>,
        setup_token: String,
    ) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
        add_claude_account_for_login(&self.state, label, setup_token).await
    }

    pub async fn start_cursor_login_session(&self) -> StartedLoginSession {
        start_cursor_login_session(&self.state).await
    }

    pub async fn cursor_login_status(
        &self,
        login_id: &str,
    ) -> Option<provider_accounts::CursorLoginStatus> {
        cursor_login_status(&self.state, login_id).await
    }

    pub async fn set_cursor_login_error(&self, login_id: &str, error: String) {
        set_cursor_login_error(&self.state, login_id, error).await;
    }

    pub async fn update_cursor_login_auth_url(&self, login_id: &str, auth_url: String) {
        update_cursor_login_auth_url(&self.state, login_id, auth_url).await;
    }

    pub async fn finish_cursor_login_session(
        &self,
        login_id: &str,
        status: String,
        account_id: Option<String>,
        error: Option<String>,
        observed_auth_url: Option<String>,
    ) {
        finish_cursor_login_session(
            &self.state,
            login_id,
            status,
            account_id,
            error,
            observed_auth_url,
        )
        .await;
    }

    pub async fn resolve_cursor_login_runtime(
        &self,
    ) -> anyhow::Result<ProviderLoginRuntimeCommand> {
        resolve_cursor_login_runtime(&self.state).await
    }

    pub async fn add_cursor_oauth_account_for_login(
        &self,
        label: Option<String>,
        auth_token: String,
        refresh_token: Option<String>,
        email: Option<String>,
    ) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
        add_cursor_oauth_account_for_login(&self.state, label, auth_token, refresh_token, email)
            .await
    }
}
