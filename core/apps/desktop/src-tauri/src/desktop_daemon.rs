use super::*;

mod auth;
mod bundle_preflight;
mod commands;
mod diagnostics;
mod health;
mod launch;
mod login_relay;
mod path_env;
mod resources;
mod systemd;
#[cfg(test)]
mod tests;

pub(super) const AVF_LINUX_HELPER_PATH_ENV: &str = "CTX_AVF_LINUX_HELPER_PATH";
pub(super) const DESKTOP_BUNDLE_DIR_ENV: &str = "CTX_BUNDLE_DIR";
const DESKTOP_DAEMON_BIN_NAME: &str = "ctx-daemon";
const AVF_GUEST_GATEWAY_HOST: &str = "192.168.64.1";
const DAEMON_AUTOMATION_ENV_BLOCKLIST: &[&str] = &[
    "AUTOMATION_LIBRARY_PATH",
    "AUTOMATION_PORT",
    "REMOTE_WEBDRIVER_URL",
    "TAURI_DRIVER_PORT",
    "TEST_RUNNER_BACKEND_PORT",
];

pub(super) use auth::{
    read_daemon_auth_with_retry, read_remote_daemon_auth_with_retry, resolve_env_local_daemon,
    resolve_existing_local_daemon,
};
pub(super) use commands::{
    desktop_daemon_request, desktop_upload_blob, DesktopDaemonRequest, DesktopHttpResponse,
};
pub(super) use health::{
    daemon_health, existing_local_daemon_matches, existing_local_daemon_matches_or_absent,
    local_daemon_health_matches_expected, normalize_daemon_pid, probe_daemon_health,
    probe_daemon_health_with_retry, probe_local_daemon_health_with_retry,
    reclaim_incompatible_local_daemon, should_reclaim_incompatible_local_daemon,
    spawned_local_daemon_incompatibility_message, terminate_pid, wait_for_daemon_reclaim,
};
pub(super) use launch::{spawn_and_validate_local_daemon, try_kill_child, SpawnedLocalDaemonReady};
pub(super) use login_relay::desktop_start_codex_login_relay;
pub(super) use resources::daemon_data_dir;
pub(super) fn desktop_bundle_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    resources::desktop_bundle_dir(app)
}
pub(super) use systemd::{stop_systemd_scope, systemd_scope_for_local_daemon_url};

pub(super) fn enforce_desktop_parity_bundle_preflight(app: &tauri::AppHandle) -> Result<()> {
    bundle_preflight::enforce_desktop_parity_bundle_preflight(
        resources::desktop_bundle_dir(app).as_deref(),
    )
}
