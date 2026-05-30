use std::sync::Arc;

use crate::daemon::managed_auto_update::ManagedDaemonAutoUpdateHost;
use crate::daemon::memleak_debug::{MemleakDebugHost, MemleakDebugHostParts};
use crate::daemon::mobile_startup::SavedMobileTunnelReconnectHost;
use crate::daemon::provider_child_reclassifier::ProviderChildReclassifierHost;
#[cfg(test)]
use crate::daemon::scheduler::DaemonSchedulerPersistenceHost;
use crate::daemon::scheduler::DaemonTerminalStateReconcileHost;
use crate::daemon::storage_guard::{StorageGuardHost, StorageGuardHostParts};
use crate::daemon::workspaces::{
    vcs_hooks::WorkspaceVcsHookHost, workspace_cache_debug_stats_host_from_runtime,
};
use crate::daemon::{
    CacheSweepHost, CacheSweepHostParts, DaemonShutdownHost, DaemonShutdownHostParts, DaemonState,
    DaemonWorktreeDataPlaneHost, ProtectedWorkspaceStoreLookup, SessionStoreLookup,
    StartupTurnReconcileHost,
};

pub(in crate::daemon) fn worktree_data_plane_host_from_state(
    state: &DaemonState,
) -> DaemonWorktreeDataPlaneHost {
    let workspace_stores = protected_workspace_store_lookup_from_state(state);
    DaemonWorktreeDataPlaneHost::new(state.global_store().clone(), workspace_stores)
}

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

pub(in crate::daemon) fn provider_child_reclassifier_host_from_state(
    state: &DaemonState,
) -> ProviderChildReclassifierHost {
    ProviderChildReclassifierHost::new(state.core.shutdown_tx.clone(), Arc::clone(&state.providers))
}

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

pub(in crate::daemon) fn managed_daemon_auto_update_host_from_state(
    state: &DaemonState,
) -> ManagedDaemonAutoUpdateHost {
    ManagedDaemonAutoUpdateHost::new(
        state.core.data_root.clone(),
        state.global_store().clone(),
        state.core.stores.clone(),
        Arc::clone(&state.core.update_drain),
    )
}

pub(in crate::daemon) fn saved_mobile_tunnel_reconnect_host_from_state(
    state: &DaemonState,
) -> SavedMobileTunnelReconnectHost {
    SavedMobileTunnelReconnectHost::new(
        state.core.auth_token.is_some(),
        state.global_store().clone(),
        state.core.daemon_url.clone(),
        state.transport.mobile_tunnel.clone(),
    )
}

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

pub(in crate::daemon) fn memleak_debug_host_from_state(state: &DaemonState) -> MemleakDebugHost {
    MemleakDebugHost::new(MemleakDebugHostParts {
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
        stores: state.core.stores.clone(),
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
