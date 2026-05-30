use std::path::PathBuf;
use std::sync::Arc;

use ctx_observability::perf_telemetry::PerfTelemetry;
use ctx_observability::telemetry::Telemetry;
use ctx_provider_runtime::ProviderRuntime;
use ctx_resource_utilization::resource_governance::ResourceGovernanceRuntime;
use ctx_resource_utilization::ResourceSampler;
use ctx_store::{Store, StoreManager};
use ctx_transport_runtime::terminals::TerminalManager;
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_runtime::HarnessRuntimeManager;
use tokio::sync::{broadcast, Mutex};

use crate::daemon::merge_queue::MergeQueueRouteHost;
use crate::daemon::state::{ProtectedWorkspaceStoreLookup, SessionRuntime};

pub(super) struct MaintenanceRouteDepsParts {
    pub(super) data_root: PathBuf,
    pub(super) global_store: Store,
    pub(super) stores: StoreManager,
    pub(super) workspace_stores: ProtectedWorkspaceStoreLookup,
    pub(super) merge_queue_host: Arc<MergeQueueRouteHost>,
    pub(super) telemetry: Telemetry,
    pub(super) perf_telemetry: PerfTelemetry,
    pub(super) resource_sampler: Arc<Mutex<ResourceSampler>>,
    pub(super) resource_governance: Arc<Mutex<ResourceGovernanceRuntime>>,
    pub(super) providers: Arc<ProviderRuntime>,
    pub(super) terminals: Arc<TerminalManager>,
    pub(super) update_drain: Arc<UpdateDrainCoordinator>,
    pub(super) sessions: Arc<SessionRuntime>,
    pub(super) harness: Arc<HarnessRuntimeManager>,
    pub(super) shutdown_tx: broadcast::Sender<()>,
    pub(super) local_shutdown_token: Option<String>,
}

#[derive(Clone)]
pub(super) struct MaintenanceRouteDeps {
    pub(super) data_root: PathBuf,
    pub(super) global_store: Store,
    pub(super) stores: StoreManager,
    pub(super) workspace_stores: ProtectedWorkspaceStoreLookup,
    pub(super) merge_queue_host: Arc<MergeQueueRouteHost>,
    pub(super) telemetry: Telemetry,
    pub(super) perf_telemetry: PerfTelemetry,
    pub(super) resource_sampler: Arc<Mutex<ResourceSampler>>,
    pub(super) resource_governance: Arc<Mutex<ResourceGovernanceRuntime>>,
    pub(super) providers: Arc<ProviderRuntime>,
    pub(super) terminals: Arc<TerminalManager>,
    pub(super) update_drain: Arc<UpdateDrainCoordinator>,
    pub(super) sessions: Arc<SessionRuntime>,
    pub(super) harness: Arc<HarnessRuntimeManager>,
    pub(super) shutdown_tx: broadcast::Sender<()>,
    pub(super) local_shutdown_token: Option<String>,
}

impl MaintenanceRouteDeps {
    pub(super) fn new(parts: MaintenanceRouteDepsParts) -> Self {
        Self {
            data_root: parts.data_root,
            global_store: parts.global_store,
            stores: parts.stores,
            workspace_stores: parts.workspace_stores,
            merge_queue_host: parts.merge_queue_host,
            telemetry: parts.telemetry,
            perf_telemetry: parts.perf_telemetry,
            resource_sampler: parts.resource_sampler,
            resource_governance: parts.resource_governance,
            providers: parts.providers,
            terminals: parts.terminals,
            update_drain: parts.update_drain,
            sessions: parts.sessions,
            harness: parts.harness,
            shutdown_tx: parts.shutdown_tx,
            local_shutdown_token: parts.local_shutdown_token,
        }
    }

    pub(super) fn workspace_store_lookup(&self) -> ProtectedWorkspaceStoreLookup {
        self.workspace_stores.clone()
    }
}
