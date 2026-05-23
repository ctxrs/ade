use std::path::Path as StdPath;

use ctx_core::ids::WorkspaceId;
use ctx_provider_install::install_state::InstallTarget;
use ctx_providers::adapters::ProviderStatus;

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
pub use auth_check::ProviderAuthCheckError;
pub use auth_import::{
    import_provider_auth_candidates, list_provider_auth_import_candidates,
    list_provider_auth_import_profiles,
};
pub use diagnostics::provider_diagnostics_snapshot;
pub(in crate::daemon) use diagnostics::provider_diagnostics_snapshot_for_runtime;
pub use harness_config::{
    mark_provider_endpoint_verification, refresh_provider_endpoint_model_catalog,
};
pub use installs::parse_provider_install_target;
pub use launch_config::{
    load_provider_launch_config_snapshot, ProviderLaunchConfigError, ProviderLaunchConfigSnapshot,
};
pub use login_sessions::{
    claim_codex_login_callback, claude_login_status, codex_login_status, codex_login_statuses,
    cursor_login_status, finish_codex_login_session, remove_codex_login_session,
    restore_codex_login_completion_token, start_codex_login_session, CodexLoginCallbackClaimError,
    StartedCodexLoginSession, StartedLoginSession,
};
pub use options::{
    effective_preferred_model_id_for_workspace, EffectivePreferredModelError,
    ProviderOptionsResponseError,
};
pub use options_cache::{store_provider_verify_cache_value, ProviderOptionsCacheSnapshot};
pub use restarts::{
    restart_codex_providers_for_auth_change, restart_provider_for_auth_change,
    stop_codex_providers_for_auth_removal,
};
pub use runtime_probe::probe_provider_auth_verification_runtime;
pub use status::{
    install_target_for_workspace, provider_status_response, providers_statuses_response,
    refresh_provider_statuses, ProviderStatusResponseError,
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
}
