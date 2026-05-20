use std::path::Path as StdPath;

use ctx_core::ids::WorkspaceId;
use ctx_harness_sources as harness_sources;
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_providers::adapters::ProviderStatus;
use serde_json::Value;
use tokio::sync::broadcast;

use super::handle::ProvidersHandle;
mod accounts;
mod admin_routes;
mod auth_check;
mod auth_import;
mod bootstrap;
mod browser_logins;
mod claude_setup_token_login;
mod codex_app_login;
mod cursor_process_login;
mod diagnostics;
mod harness_config;
mod installs;
mod inventory;
mod kimi_oauth_login;
mod launch_config;
mod login_routes;
mod login_runtime;
mod login_sessions;
mod options;
mod options_cache;
mod restarts;
mod runtime_probe;
mod status;
mod usage;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use accounts::persist_successful_codex_login;
pub use accounts::{
    AmpAccountUpsertRouteRequest, AmpAccountsResponse, ClaudeAccountUpsertRouteRequest,
    ClaudeAccountsResponse, CodexAccountsResponse, CodexHostImportProbeRouteResponse,
    CodexHostImportRouteRequest, CopilotAccountUpsertRouteRequest, CopilotAccountsResponse,
    CursorAccountUpsertRouteRequest, CursorAccountsResponse, GeminiAccountUpsertRouteRequest,
    GeminiAccountsResponse, KimiAccountUpsertRouteRequest, KimiAccountsResponse,
    MistralAccountUpsertRouteRequest, MistralAccountsResponse, ProviderAccountRouteError,
    ProviderAccountRouteErrorKind, ProviderActiveAccountRouteRequest,
    QwenAccountUpsertRouteRequest, QwenAccountsResponse,
};
pub use admin_routes::{
    ProviderAdminRouteError, ProviderAdminRouteErrorKind, ProviderDevRestartRouteRequest,
    ProviderDevRestartRouteResponse, ProviderMatrixRefreshRouteResponse,
};
pub use auth_check::{
    authenticate_provider_for_workspace, verify_provider_for_workspace,
    AuthenticateProviderForWorkspaceRouteBody, AuthenticateProviderForWorkspaceRouteRequest,
    ProviderAuthCheckError, ProviderAuthCheckRouteError, ProviderAuthCheckRouteErrorStatus,
    ProviderAuthCheckRouteResponse, ProviderAuthCheckSnapshot,
    VerifyProviderForWorkspaceRouteRequest,
};
pub use auth_import::provider_auth_import_result_requires_restart;
pub use auth_import::{
    import_provider_auth_candidates, list_provider_auth_import_candidates,
    list_provider_auth_import_profiles, ProviderAuthImportCandidatesRouteResponse,
    ProviderAuthImportProfilesRouteResponse, ProviderAuthImportRouteError,
    ProviderAuthImportRouteRequest, ProviderAuthImportRouteResponse,
};
pub use bootstrap::{
    ProvidersBootstrapResponse, ProvidersBootstrapRouteError, ProvidersBootstrapRouteErrorKind,
    ProvidersBootstrapRouteRequest,
};
pub use claude_setup_token_login::{
    ClaudeLoginRouteError, ClaudeLoginRouteErrorKind, ClaudeLoginStartRouteRequest,
    ClaudeLoginStartRouteResponse, ClaudeLoginStatusRouteResponse,
};
pub use codex_app_login::{
    CodexLoginCompleteRouteRequest, CodexLoginCompleteRouteResponse, CodexLoginRouteError,
    CodexLoginRouteErrorKind, CodexLoginStartRouteRequest, CodexLoginStartRouteResponse,
    CodexLoginStatusRouteResponse,
};
pub use cursor_process_login::{
    CursorLoginRouteError, CursorLoginRouteErrorKind, CursorLoginStartRouteRequest,
    CursorLoginStartRouteResponse, CursorLoginStatusRouteResponse,
};
pub use diagnostics::provider_diagnostics_snapshot;
pub use harness_config::{
    delete_provider_harness_endpoint, get_provider_harness_config,
    mark_provider_endpoint_verification, refresh_provider_endpoint_model_catalog,
    refresh_provider_harness_endpoint_models, select_provider_harness_source,
    set_provider_harness_endpoint_manual_models, upsert_provider_harness_endpoint,
    ProviderHarnessConfigRouteError, ProviderHarnessEndpointRouteError,
    ProviderHarnessEndpointRouteErrorKind, ProviderHarnessSourceConfig,
    SelectProviderHarnessSourceRouteRequest, SetProviderHarnessEndpointManualModelsRouteRequest,
    UpsertProviderHarnessEndpointRouteRequest,
};
pub use installs::{
    cancel_provider_install, get_provider_install_info, list_provider_install_events,
    parse_provider_install_target, provider_install_event_sender, start_all_provider_installs,
    start_provider_install, ProviderInstallEventStreamRoute, ProviderInstallInfo,
    ProviderInstallJsonRouteError, ProviderInstallJsonRouteErrorStatus,
    ProviderInstallProgressEvent, ProviderInstallStartRouteResponse,
    ProviderInstallStatusOnlyRouteError, ProviderInstallStatusesRouteRequest,
    ProviderInstallStatusesRouteResponse, StartProviderInstallError,
};
pub use launch_config::{
    load_provider_launch_config_snapshot, ProviderLaunchConfigError, ProviderLaunchConfigSnapshot,
};
pub use login_routes::{
    AmpLoginStatusRouteResponse, GeminiLoginStatusRouteResponse, KimiLoginStatusRouteResponse,
    MistralLoginStatusRouteResponse, ProviderLoginRouteError, ProviderLoginRouteErrorKind,
    ProviderLoginStartRouteRequest, ProviderLoginStartRouteResponse, QwenLoginStatusRouteResponse,
};
pub use login_runtime::{resolve_claude_login_runtime_from_config, ProviderLoginRuntimeCommand};
pub use login_sessions::{
    claim_codex_login_callback, claude_login_status, codex_login_status, codex_login_statuses,
    cursor_login_status, finish_codex_login_session, remove_codex_login_session,
    restore_codex_login_completion_token, start_codex_login_session, CodexLoginCallbackClaimError,
    StartedCodexLoginSession, StartedLoginSession,
};
pub use options::{
    effective_preferred_model_id_for_workspace, get_provider_options_response,
    EffectivePreferredModelError, ProviderOptionsResponseError, ProviderOptionsRouteError,
    ProviderOptionsRouteErrorStatus, ProviderOptionsRouteRequest,
};
pub use options_cache::{store_provider_verify_cache_value, ProviderOptionsCacheSnapshot};
pub use restarts::{
    restart_codex_providers_for_auth_change, restart_provider_for_auth_change,
    stop_codex_providers_for_auth_removal,
};
pub use runtime_probe::probe_provider_auth_verification_runtime;
pub use status::{
    install_target_for_workspace, provider_status_response, providers_statuses_response,
    refresh_provider_statuses, ProviderStatusListRouteError, ProviderStatusResponseError,
    ProviderStatusRouteError, ProviderStatusRouteErrorKind, ProviderStatusRouteQuery,
};
pub use usage::{
    CodexAccountsUsageRouteResponse, ProviderUsageRouteError, ProviderUsageRouteQuery,
    ProviderUsageRouteSnapshot,
};

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

    #[cfg(feature = "test-support")]
    pub async fn remove_codex_account_for_test(&self, account_id: &str) -> anyhow::Result<()> {
        accounts::remove_codex_account(&self.state, account_id)
            .await
            .map(|_| ())
            .map_err(|err| anyhow::anyhow!(err.to_string()))
    }
}
