use std::sync::Arc;

use crate::daemon::{
    lifecycle, managed_auto_update, memleak_debug, merge_queue, mobile_startup,
    provider_child_reclassifier, provider_guard, provider_restart, provider_usage,
    resource_telemetry, storage_guard, DaemonState,
};

pub(super) fn spawn_daemon_background_services(
    state: Arc<DaemonState>,
    requested_binds: Vec<String>,
) {
    resource_telemetry::spawn_resource_telemetry(state.clone());
    memleak_debug::spawn_memleak_debug(state.clone());
    storage_guard::spawn_storage_guard(state.clone());
    provider_guard::spawn_provider_guard(state.clone());
    provider_restart::spawn_provider_restart(state.clone());
    provider_child_reclassifier::spawn_provider_child_reclassifier(state.clone());
    merge_queue::spawn_merge_queue_runner(state.clone());
    provider_usage::spawn_provider_usage_poller(state.clone());
    managed_auto_update::spawn_managed_daemon_auto_update(state.clone(), requested_binds);
    lifecycle::spawn_process_shutdown_listener(state.clone());

    mobile_startup::spawn_saved_mobile_tunnel_reconnect(state.clone());

    spawn_startup_provider_status_refresh(state);
}

pub(in crate::daemon) fn spawn_startup_provider_status_refresh(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        if let Err(err) = crate::daemon::providers::refresh_provider_statuses(state.as_ref()).await
        {
            tracing::warn!("startup provider status refresh failed: {err:#}");
        }
    });
}
