mod accounts;
mod auth;
mod bootstrap;
mod diagnostics;
mod harness_config;
mod installs;
mod launch_config;
mod login_runtime;
mod login_sessions;
mod options_cache;
mod restarts;
mod runtime_probe;
mod status;
mod usage;

pub(crate) use accounts::{
    add_claude_account, add_claude_account_for_login, add_copilot_account, add_cursor_account,
    add_cursor_oauth_account_for_login, add_gemini_account, add_gemini_account_for_login,
    add_kimi_account, add_kimi_oauth_account_for_login, add_qwen_account,
    add_qwen_account_for_login, amp_login_provider_env,
    ensure_amp_account_registry_from_runtime_auth, gemini_login_auth_method_id,
    gemini_login_provider_env, import_host_codex_auth, load_amp_account_registry,
    load_claude_account_registry, load_codex_account_registry, load_codex_accounts_snapshot,
    load_copilot_account_registry, load_cursor_account_registry, load_gemini_account_registry,
    load_kimi_account_registry, load_mistral_account_registry, load_qwen_account_registry,
    mistral_login_provider_env, persist_successful_codex_login, prepare_amp_login_paths,
    prepare_codex_login_start, prepare_gemini_login_paths, prepare_mistral_login_paths,
    prepare_qwen_login_paths, probe_host_codex_auth_candidate, qwen_login_provider_env,
    remove_amp_account, remove_claude_account, remove_codex_account, remove_copilot_account,
    remove_cursor_account, remove_gemini_account, remove_kimi_account, remove_mistral_account,
    remove_qwen_account, set_active_amp_account, set_active_claude_account,
    set_active_codex_account, set_active_copilot_account, set_active_cursor_account,
    set_active_gemini_account, set_active_kimi_account, set_active_mistral_account,
    set_active_qwen_account, upsert_amp_account, upsert_amp_account_for_login,
    upsert_mistral_account, upsert_mistral_account_for_login, CodexAccountsSnapshot,
    PreparedAmpLoginPaths, PreparedCodexLoginStart, PreparedGeminiLoginPaths,
    PreparedMistralLoginPaths, PreparedQwenLoginPaths, ProviderAccountMutationError,
};
pub(crate) use auth::{
    authenticate_provider_for_workspace_runtime, ProviderWorkspaceAuthenticationError,
};
pub(crate) use bootstrap::{build_bootstrap_options, visible_provider_count_hint};
pub(crate) use diagnostics::provider_diagnostics_snapshot;
pub(crate) use harness_config::{
    delete_provider_harness_endpoint, get_provider_harness_config,
    mark_provider_endpoint_verification, refresh_provider_endpoint_model_catalog,
    refresh_provider_harness_endpoint_models, select_provider_harness_source,
    set_provider_harness_endpoint_manual_models, upsert_provider_harness_endpoint,
};
pub(crate) use installs::{
    cancel_provider_install, get_provider_install_info, list_provider_install_events,
    parse_provider_install_target, provider_install_event_sender, start_all_provider_installs,
    start_provider_install, StartProviderInstallError,
};
pub(crate) use launch_config::{
    load_provider_launch_config_snapshot, ProviderLaunchConfigError, ProviderLaunchConfigSnapshot,
};
pub(crate) use login_runtime::{
    resolve_claude_login_runtime, resolve_cursor_login_runtime, ProviderLoginRuntimeCommand,
};
#[cfg(test)]
pub(crate) use login_runtime::{
    resolve_claude_login_runtime_from_config, resolve_cursor_login_runtime_from_config,
};
pub(crate) use login_sessions::{
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
    update_cursor_login_auth_url, CodexLoginCallbackClaimError,
};
pub(crate) use options_cache::{store_provider_verify_cache_value, ProviderOptionsCacheSnapshot};
pub(crate) use restarts::{
    restart_codex_providers_for_auth_change, restart_provider_for_auth_change,
};
pub(crate) use runtime_probe::{
    probe_provider_auth_verification_runtime, probe_provider_options_env,
    probe_runtime_models_for_provider_options, probe_selected_endpoint_runtime_launch,
    provider_has_active_auth_for_workspace_runtime,
};
pub(crate) use status::{
    install_target_for_workspace, provider_status_response, providers_statuses_response,
    refresh_provider_statuses, ProviderStatusResponseError,
};
pub(crate) use usage::{load_codex_accounts_usage, load_provider_usage};
