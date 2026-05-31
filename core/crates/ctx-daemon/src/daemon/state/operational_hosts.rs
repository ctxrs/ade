use std::sync::Arc;

use crate::daemon::managed_auto_update::ManagedDaemonAutoUpdateHost;
use crate::daemon::memleak_debug::{MemleakDebugHost, MemleakDebugHostParts};
use crate::daemon::merge_queue::MergeQueueRouteHost;
use crate::daemon::mobile_startup::SavedMobileTunnelReconnectHost;
use crate::daemon::provider_capability_hosts::ProviderLifecycleBackgroundHost;
use crate::daemon::provider_child_reclassifier::ProviderChildReclassifierHost;
#[cfg(test)]
use crate::daemon::scheduler::DaemonSchedulerPersistenceHost;
use crate::daemon::scheduler::DaemonTerminalStateReconcileHost;
use crate::daemon::storage_guard::{StorageGuardHost, StorageGuardHostParts};
#[cfg(test)]
use crate::daemon::workspaces::vcs_hooks::WorkspaceVcsHookHost;
use crate::daemon::workspaces::workspace_cache_debug_stats_host_from_runtime;
use crate::daemon::{
    CacheSweepHost, CacheSweepHostParts, DaemonShutdownHost, DaemonShutdownHostParts,
    DaemonShutdownSignal, DaemonState, ProtectedWorkspaceStoreLookup, SessionStoreLookup,
    StartupTurnReconcileHost,
};

use super::merge_queue::{merge_queue_route_host_from_parts, MergeQueueRouteHostParts};

pub(in crate::daemon) struct DaemonOperationalHosts {
    pub(in crate::daemon) cache_sweep: CacheSweepHost,
    pub(in crate::daemon) memleak_debug: MemleakDebugHost,
    pub(in crate::daemon) provider_lifecycle_background: Arc<ProviderLifecycleBackgroundHost>,
    pub(in crate::daemon) provider_child_reclassifier: Arc<ProviderChildReclassifierHost>,
    pub(in crate::daemon) startup_turn_reconcile: StartupTurnReconcileHost,
    pub(in crate::daemon) storage_guard: StorageGuardHost,
    pub(in crate::daemon) merge_queue: Arc<MergeQueueRouteHost>,
    pub(in crate::daemon) managed_daemon_auto_update: ManagedDaemonAutoUpdateHost,
    pub(in crate::daemon) daemon_shutdown: DaemonShutdownHost,
    pub(in crate::daemon) saved_mobile_tunnel_reconnect: SavedMobileTunnelReconnectHost,
    pub(in crate::daemon) shutdown_signal: DaemonShutdownSignal,
}

impl DaemonOperationalHosts {
    pub(in crate::daemon) fn from_state(state: &DaemonState) -> Self {
        let global_store = state.global_store().clone();
        let stores = state.core.stores.clone();
        let workspace_stores = protected_workspace_store_lookup_from_state(state);
        let session_stores =
            SessionStoreLookup::new(global_store.clone(), workspace_stores.clone());
        let terminal_state_reconcile = DaemonTerminalStateReconcileHost::new(
            session_stores.clone(),
            state.session_publication.clone(),
            state.task_session_cleanup.clone(),
        );
        let startup_turn_reconcile = StartupTurnReconcileHost::new(
            global_store.clone(),
            stores.clone(),
            terminal_state_reconcile,
        );
        let merge_queue = merge_queue_route_host_from_parts(MergeQueueRouteHostParts {
            stores: stores.clone(),
            global_store: global_store.clone(),
            workspace_stores: workspace_stores.clone(),
            session_stores: session_stores.clone(),
            merge_queue: Arc::clone(&state.transport.merge_queue),
            ops_events: state.telemetry.ops_events.clone(),
            session_publication: state.session_publication.clone(),
        });
        Self {
            cache_sweep: CacheSweepHost::new(CacheSweepHostParts {
                sessions: Arc::clone(&state.sessions),
                file_completions_cache: Arc::clone(&state.workspaces.file_completions_cache),
                workspace_file_completions_cache: Arc::clone(
                    &state.workspaces.workspace_file_completions_cache,
                ),
                git_status_snapshots: Arc::clone(&state.workspaces.git_status_snapshots),
                worktree_vcs_snapshots: Arc::clone(&state.workspaces.worktree_vcs_snapshots),
                workspace_active_snapshot_cache: Arc::clone(
                    &state.workspaces.workspace_active_snapshot_cache,
                ),
                workspace_active_heads_cache: Arc::clone(
                    &state.workspaces.workspace_active_heads_cache,
                ),
                worktree_bootstrap_gates: Arc::clone(&state.workspaces.worktree_bootstrap_gates),
                workspace_stores: workspace_stores.clone(),
                perf_telemetry: state.telemetry.perf_telemetry.clone(),
                shutdown_tx: state.core.shutdown_tx.clone(),
            }),
            memleak_debug: MemleakDebugHost::new(MemleakDebugHostParts {
                data_root: state.core.data_root.clone(),
                shutdown_tx: state.core.shutdown_tx.clone(),
                sessions: Arc::clone(&state.sessions),
                workspaces: workspace_cache_debug_stats_host_from_runtime(&state.workspaces),
                providers: Arc::clone(&state.providers),
                active_snapshot: Arc::clone(&state.workspaces.workspace_active_snapshot),
                terminals: Arc::clone(&state.transport.terminals),
                perf_telemetry: state.telemetry.perf_telemetry.clone(),
                web_sessions: Arc::clone(&state.transport.web_sessions),
                harness_runtime: Arc::clone(&state.execution.harness),
                stores: stores.clone(),
            }),
            provider_lifecycle_background: Arc::clone(&state.provider_lifecycle_background),
            provider_child_reclassifier: Arc::new(ProviderChildReclassifierHost::new(
                state.core.shutdown_tx.clone(),
                Arc::clone(&state.providers),
            )),
            startup_turn_reconcile,
            storage_guard: StorageGuardHost::new(StorageGuardHostParts {
                data_root: state.core.data_root.clone(),
                storage_guard: Arc::clone(&state.core.storage_guard),
                resource_sampler: Arc::clone(&state.telemetry.resource_sampler),
                sessions: Arc::clone(&state.sessions),
                session_stores: session_stores.clone(),
                ops_events: state.telemetry.ops_events.clone(),
                shutdown_tx: state.core.shutdown_tx.clone(),
            }),
            merge_queue,
            managed_daemon_auto_update: ManagedDaemonAutoUpdateHost::new(
                state.core.data_root.clone(),
                global_store,
                stores,
                Arc::clone(&state.core.update_drain),
            ),
            daemon_shutdown: DaemonShutdownHost::new(DaemonShutdownHostParts {
                global_store: state.global_store().clone(),
                stores: state.core.stores.clone(),
                session_stores,
                session_lifecycle: Arc::clone(&state.sessions),
                session_publication: state.session_publication.clone(),
                provider_lifecycle: Arc::clone(&state.providers),
                update_drain: Arc::clone(&state.core.update_drain),
                substrate_lifecycle: Arc::clone(&state.execution.harness),
                shutdown_signal: state.core.shutdown_tx.clone(),
            }),
            saved_mobile_tunnel_reconnect: SavedMobileTunnelReconnectHost::new(
                state.core.auth_token.is_some(),
                state.global_store().clone(),
                state.core.daemon_url.clone(),
                state.transport.mobile_tunnel.clone(),
            ),
            shutdown_signal: DaemonShutdownSignal::new(state.core.shutdown_tx.clone()),
        }
    }
}

#[cfg(test)]
pub(in crate::daemon) fn workspace_vcs_hook_host_from_state(
    state: &DaemonState,
) -> WorkspaceVcsHookHost {
    let workspace_stores = protected_workspace_store_lookup_from_state(state);
    WorkspaceVcsHookHost::new(
        state.core.data_root.clone(),
        state.core.daemon_url.clone(),
        state.global_store().clone(),
        workspace_stores,
        Arc::clone(&state.execution.harness),
    )
}

pub(in crate::daemon) fn terminal_state_reconcile_host_from_state(
    state: &DaemonState,
) -> DaemonTerminalStateReconcileHost {
    let workspace_stores = protected_workspace_store_lookup_from_state(state);
    let session_stores = SessionStoreLookup::new(state.global_store().clone(), workspace_stores);
    DaemonTerminalStateReconcileHost::new(
        session_stores,
        state.session_publication.clone(),
        state.task_session_cleanup.clone(),
    )
}

#[cfg(test)]
pub(in crate::daemon) fn startup_turn_reconcile_host_from_state(
    state: &DaemonState,
) -> StartupTurnReconcileHost {
    StartupTurnReconcileHost::new(
        state.global_store().clone(),
        state.core.stores.clone(),
        terminal_state_reconcile_host_from_state(state),
    )
}

#[cfg(test)]
pub(in crate::daemon) fn scheduler_persistence_host_from_state(
    state: &DaemonState,
) -> DaemonSchedulerPersistenceHost {
    let workspace_stores = protected_workspace_store_lookup_from_state(state);
    let session_stores = SessionStoreLookup::new(state.global_store().clone(), workspace_stores);
    DaemonSchedulerPersistenceHost::new(session_stores, state.session_publication.clone())
}

#[cfg(test)]
pub(in crate::daemon) fn daemon_shutdown_host_from_state(
    state: &DaemonState,
) -> DaemonShutdownHost {
    let workspace_stores = protected_workspace_store_lookup_from_state(state);
    let session_stores = SessionStoreLookup::new(state.global_store().clone(), workspace_stores);
    DaemonShutdownHost::new(DaemonShutdownHostParts {
        global_store: state.global_store().clone(),
        stores: state.core.stores.clone(),
        session_stores,
        session_lifecycle: Arc::clone(&state.sessions),
        session_publication: state.session_publication.clone(),
        provider_lifecycle: Arc::clone(&state.providers),
        update_drain: Arc::clone(&state.core.update_drain),
        substrate_lifecycle: Arc::clone(&state.execution.harness),
        shutdown_signal: state.core.shutdown_tx.clone(),
    })
}

#[cfg(test)]
pub(in crate::daemon) fn storage_guard_host_from_state(state: &DaemonState) -> StorageGuardHost {
    let workspace_stores = protected_workspace_store_lookup_from_state(state);
    let session_stores = SessionStoreLookup::new(state.global_store().clone(), workspace_stores);
    StorageGuardHost::new(StorageGuardHostParts {
        data_root: state.core.data_root.clone(),
        storage_guard: Arc::clone(&state.core.storage_guard),
        resource_sampler: Arc::clone(&state.telemetry.resource_sampler),
        sessions: Arc::clone(&state.sessions),
        session_stores,
        ops_events: state.telemetry.ops_events.clone(),
        shutdown_tx: state.core.shutdown_tx.clone(),
    })
}

#[cfg(test)]
pub(in crate::daemon) fn cache_sweep_host_from_state(state: &DaemonState) -> CacheSweepHost {
    CacheSweepHost::new(CacheSweepHostParts {
        sessions: Arc::clone(&state.sessions),
        file_completions_cache: Arc::clone(&state.workspaces.file_completions_cache),
        workspace_file_completions_cache: Arc::clone(
            &state.workspaces.workspace_file_completions_cache,
        ),
        git_status_snapshots: Arc::clone(&state.workspaces.git_status_snapshots),
        worktree_vcs_snapshots: Arc::clone(&state.workspaces.worktree_vcs_snapshots),
        workspace_active_snapshot_cache: Arc::clone(
            &state.workspaces.workspace_active_snapshot_cache,
        ),
        workspace_active_heads_cache: Arc::clone(&state.workspaces.workspace_active_heads_cache),
        worktree_bootstrap_gates: Arc::clone(&state.workspaces.worktree_bootstrap_gates),
        workspace_stores: protected_workspace_store_lookup_from_state(state),
        perf_telemetry: state.telemetry.perf_telemetry.clone(),
        shutdown_tx: state.core.shutdown_tx.clone(),
    })
}

fn protected_workspace_store_lookup_from_state(
    state: &DaemonState,
) -> ProtectedWorkspaceStoreLookup {
    ProtectedWorkspaceStoreLookup::new(
        state.core.stores.clone(),
        Arc::clone(&state.sessions),
        Arc::clone(&state.transport.merge_queue),
    )
}
