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
mod login_deps;
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
pub use restarts::restart_provider_for_auth_change;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use restarts::restart_provider_for_auth_change_with_runtime;
pub use runtime_probe::probe_provider_auth_verification_runtime;
pub use status::{
    install_target_for_workspace, provider_status_response, providers_statuses_response,
    refresh_provider_statuses, ProviderStatusResponseError,
};
