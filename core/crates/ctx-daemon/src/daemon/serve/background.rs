use std::sync::Arc;

use crate::daemon::managed_auto_update::ManagedDaemonAutoUpdateHost;
use crate::daemon::merge_queue::MergeQueueRouteHost;
use crate::daemon::mobile_startup::SavedMobileTunnelReconnectHost;
use crate::daemon::provider_capability_hosts::ProviderLifecycleBackgroundHost;
use crate::daemon::provider_child_reclassifier::ProviderChildReclassifierHost;
use crate::daemon::storage_guard::StorageGuardHost;
use crate::daemon::{
    lifecycle, managed_auto_update, merge_queue, mobile_startup, provider_child_reclassifier,
    provider_guard, provider_restart, provider_usage, resource_telemetry, storage_guard,
    DaemonShutdownHost, ProviderStatusHandle, ProviderUsageHandle,
};

pub(super) fn spawn_provider_background_services(
    provider_lifecycle_background: Arc<ProviderLifecycleBackgroundHost>,
    provider_child_reclassifier_host: Arc<ProviderChildReclassifierHost>,
    provider_status: ProviderStatusHandle,
    provider_usage: ProviderUsageHandle,
) {
    resource_telemetry::spawn_resource_telemetry(Arc::clone(&provider_lifecycle_background));
    provider_guard::spawn_provider_guard(Arc::clone(&provider_lifecycle_background));
    provider_restart::spawn_provider_restart(provider_lifecycle_background);
    provider_child_reclassifier::spawn_provider_child_reclassifier(
        provider_child_reclassifier_host,
    );
    provider_usage::spawn_provider_usage_poller(Arc::new(provider_usage));
    spawn_startup_provider_status_refresh(provider_status);
}

pub(super) fn spawn_operational_background_services(
    storage_guard_host: StorageGuardHost,
    merge_queue_host: Arc<MergeQueueRouteHost>,
    managed_auto_update_host: ManagedDaemonAutoUpdateHost,
    shutdown_host: DaemonShutdownHost,
    saved_mobile_tunnel_reconnect_host: SavedMobileTunnelReconnectHost,
    requested_binds: Vec<String>,
) {
    storage_guard::spawn_storage_guard(storage_guard_host);
    merge_queue::spawn_merge_queue_runner(merge_queue_host);
    managed_auto_update::spawn_managed_daemon_auto_update(
        managed_auto_update_host,
        requested_binds,
    );
    lifecycle::spawn_process_shutdown_listener(shutdown_host);

    mobile_startup::spawn_saved_mobile_tunnel_reconnect(saved_mobile_tunnel_reconnect_host);
}

pub(in crate::daemon) fn spawn_startup_provider_status_refresh(handle: ProviderStatusHandle) {
    tokio::spawn(async move {
        if let Err(err) =
            ctx_provider_runtime::provider_status_service::refresh_provider_statuses(&handle).await
        {
            tracing::warn!("startup provider status refresh failed: {err:#}");
        }
    });
}
