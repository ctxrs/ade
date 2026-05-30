use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{broadcast, watch, Mutex};

use crate::daemon::scheduler::SchedulerCommand;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_observability::ops_events::{OpsEvent, OpsEvents};
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use ctx_observability::telemetry::Telemetry;
use ctx_provider_install::install_state::{
    InstallErrorCode, InstallId, InstallProgressEvent, InstallStateKind, InstallTarget,
};
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::ask_user_question::AskUserQuestionBroker;
use ctx_resource_utilization::resource_governance::ResourceGovernanceRuntime;
use ctx_resource_utilization::ResourceSampler;
use ctx_store::{Store, StoreManager};
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;
use ctx_transport_runtime::terminals::TerminalManager;
use ctx_transport_runtime::web_sessions::WebSessionManager;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_runtime::HarnessRuntimeManager;

mod builder;
mod cache;
mod installs;
mod merge_queue;
mod metrics;
mod operational_hosts;
mod runtime_adapters;
mod store_lookup;
mod types;
mod worktree_data_plane;

pub use cache::{CacheSweepConfig, TimedEntry};
pub(in crate::daemon) use cache::{CacheSweepHost, CacheSweepHostParts};
pub(in crate::daemon) use merge_queue::merge_queue_route_host_from_state;
#[cfg(test)]
pub(in crate::daemon) use operational_hosts::scheduler_persistence_host_from_state;
pub(in crate::daemon) use operational_hosts::{
    cache_sweep_host_from_state, daemon_shutdown_host_from_state,
    managed_daemon_auto_update_host_from_state, provider_child_reclassifier_host_from_state,
    saved_mobile_tunnel_reconnect_host_from_state, startup_turn_reconcile_host_from_state,
    storage_guard_host_from_state, terminal_state_reconcile_host_from_state,
    workspace_vcs_hook_host_from_state, worktree_data_plane_host_from_state,
};
use runtime_adapters::{
    CtxExecutionHarness, CtxRuntimeEventSink, CtxRuntimeMetricsSink, DefaultWarmupOperations,
};
pub(in crate::daemon) use worktree_data_plane::DaemonWorktreeDataPlaneHost;

pub(in crate::daemon) use store_lookup::{
    session_store_access_anyhow, ProtectedWorkspaceStoreLookup, SessionStoreLookup,
    TaskStoreLookup, WeakSessionStoreLookup,
};
pub use store_lookup::{SessionStoreAccessError, WorkspaceStoreAccessError};
pub use types::WorktreeBootstrapGate;
pub use types::{
    AppRuntimeFlags, CoreState, DaemonState, ExecutionRuntime, ProviderRuntime, SessionRuntime,
    StoreLookup, TelemetryRuntime, TransportRuntime, WorkspaceRuntime,
};
pub(crate) use types::{
    WorkspaceActiveHeadsCache, WorkspaceActiveSnapshotCache, WorkspaceFileCompletionsCache,
    WorktreeBootstrapGateCache, WorktreeFileCompletionsCache, WorktreeGitStatusSnapshotCache,
    WorktreeVcsSnapshotCache,
};

#[cfg(test)]
mod tests;
