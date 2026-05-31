use std::path::PathBuf;
use std::sync::Arc;

use crate::daemon::git_status::{WorktreeVcsExecutionHost, WorktreeVcsRuntimeHost};
use crate::daemon::merge_queue::MergeQueueRouteHost;
use crate::daemon::scheduler::SessionSchedulerWorkerHost;
use crate::daemon::state::{
    DaemonState, ProtectedWorkspaceStoreLookup, SessionRuntime, SessionStoreLookup,
    WeakSessionStoreLookup, WorkspaceActiveHeadsCache, WorkspaceActiveSnapshotCache,
    WorkspaceFileCompletionsCache, WorktreeBootstrapGateCache, WorktreeFileCompletionsCache,
};
use crate::daemon::task_session_effects::TaskPublicationHost;
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_mcp_auth::McpAuthRegistry;
use ctx_merge_queue::MergeQueueRuntime;
use ctx_observability::ops_events::OpsEvents;
use ctx_observability::perf_telemetry::PerfTelemetry;
use ctx_observability::provider_unknown_events::ProviderUnknownEvents;
use ctx_observability::telemetry::Telemetry;
use ctx_provider_runtime::ProviderRuntime;
use ctx_providers::ask_user_question::AskUserQuestionBroker;
use ctx_resource_utilization::resource_governance::ResourceGovernanceRuntime;
use ctx_resource_utilization::ResourceSampler;
use ctx_storage_admission::StorageGuardRuntime;
use ctx_store::{Store, StoreManager};
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;
use ctx_transport_runtime::terminals::TerminalManager;
use ctx_transport_runtime::web_sessions::WebSessionManager;
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_runtime::HarnessRuntimeManager;

#[derive(Clone)]
pub(super) struct RouteGraphParts {
    pub(super) data_root: PathBuf,
    pub(super) tool_output_spool_dir: PathBuf,
    pub(super) daemon_url: String,
    pub(super) public_base_url: Option<String>,
    pub(super) auth_token: Option<String>,
    pub(super) local_shutdown_token: Option<String>,
    pub(super) mcp_auth: Arc<McpAuthRegistry>,
    pub(super) storage_guard: Arc<StorageGuardRuntime>,
    pub(super) ask_user_question: Arc<AskUserQuestionBroker>,
    pub(super) global_store: Store,
    pub(super) stores: StoreManager,
    pub(super) update_drain: Arc<UpdateDrainCoordinator>,
    pub(super) shutdown_tx: tokio::sync::broadcast::Sender<()>,
    pub(super) sessions: Arc<SessionRuntime>,
    pub(super) scheduler_worker_host: Arc<SessionSchedulerWorkerHost>,
    pub(super) providers: Arc<ProviderRuntime>,
    pub(super) telemetry: Telemetry,
    pub(super) ops_events: OpsEvents,
    pub(super) perf_telemetry: PerfTelemetry,
    pub(super) provider_unknown_events: ProviderUnknownEvents,
    pub(super) resource_governance: Arc<tokio::sync::Mutex<ResourceGovernanceRuntime>>,
    pub(super) resource_sampler: Arc<tokio::sync::Mutex<ResourceSampler>>,
    pub(super) terminals: Arc<TerminalManager>,
    pub(super) mobile_tunnel: MobileTunnelManager,
    pub(super) web_sessions: Arc<WebSessionManager>,
    pub(super) merge_queue: Arc<MergeQueueRuntime>,
    pub(super) harness: Arc<HarnessRuntimeManager>,
    pub(super) execution_setup: Arc<ExecutionSetupCoordinator>,
    pub(super) active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    pub(super) workspace_active_snapshot_cache: WorkspaceActiveSnapshotCache,
    pub(super) workspace_active_heads_cache: WorkspaceActiveHeadsCache,
    pub(super) workspace_file_completions_cache: WorkspaceFileCompletionsCache,
    pub(super) worktree_file_completions_cache: WorktreeFileCompletionsCache,
    pub(super) worktree_bootstrap_gates: WorktreeBootstrapGateCache,
    pub(super) attachment_materialization:
        Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentMaterializationRuntime>,
    pub(super) workspace_stores: ProtectedWorkspaceStoreLookup,
    pub(super) session_stores: SessionStoreLookup,
    pub(super) weak_session_stores: WeakSessionStoreLookup,
    pub(super) merge_queue_host: Arc<MergeQueueRouteHost>,
    pub(super) task_publication: Arc<TaskPublicationHost>,
    pub(super) worktree_vcs_runtime: WorktreeVcsRuntimeHost,
    pub(super) worktree_vcs_execution: WorktreeVcsExecutionHost,
    pub(super) workspace_vcs_stream_runtime:
        crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime,
}

impl RouteGraphParts {
    pub(super) fn from_state(state: &Arc<DaemonState>) -> Self {
        let global_store = state.global_store().clone();
        let stores = state.core.stores.clone();
        let workspace_stores = ProtectedWorkspaceStoreLookup::new(
            stores.clone(),
            Arc::clone(&state.sessions),
            Arc::clone(&state.transport.merge_queue),
        );
        let session_stores =
            SessionStoreLookup::new(global_store.clone(), workspace_stores.clone());
        let merge_queue_host = crate::daemon::state::merge_queue_route_host_from_parts(
            crate::daemon::state::MergeQueueRouteHostParts {
                stores: stores.clone(),
                global_store: global_store.clone(),
                workspace_stores: workspace_stores.clone(),
                session_stores: session_stores.clone(),
                merge_queue: Arc::clone(&state.transport.merge_queue),
                ops_events: state.telemetry.ops_events.clone(),
                session_publication: state.session_publication.clone(),
            },
        );
        let weak_session_stores = WeakSessionStoreLookup::new(
            global_store.clone(),
            stores.clone(),
            Arc::downgrade(&state.sessions),
            Arc::clone(&state.transport.merge_queue),
        );

        Self {
            data_root: state.core.data_root.clone(),
            tool_output_spool_dir: state.core.tool_output_spool_dir.clone(),
            daemon_url: state.core.daemon_url.clone(),
            public_base_url: state.core.public_base_url.clone(),
            auth_token: state.core.auth_token.clone(),
            local_shutdown_token: state.core.local_shutdown_token.clone(),
            mcp_auth: Arc::clone(&state.core.mcp_auth),
            storage_guard: Arc::clone(&state.core.storage_guard),
            ask_user_question: Arc::clone(&state.core.ask_user_question),
            global_store: global_store.clone(),
            stores,
            update_drain: Arc::clone(&state.core.update_drain),
            shutdown_tx: state.core.shutdown_tx.clone(),
            sessions: Arc::clone(&state.sessions),
            scheduler_worker_host: state.session_scheduler_worker_host.worker_host(),
            providers: Arc::clone(&state.providers),
            telemetry: state.telemetry.telemetry.clone(),
            ops_events: state.telemetry.ops_events.clone(),
            perf_telemetry: state.telemetry.perf_telemetry.clone(),
            provider_unknown_events: state.telemetry.provider_unknown_events.clone(),
            resource_governance: Arc::clone(&state.telemetry.resource_governance),
            resource_sampler: Arc::clone(&state.telemetry.resource_sampler),
            terminals: Arc::clone(&state.transport.terminals),
            mobile_tunnel: state.transport.mobile_tunnel.clone(),
            web_sessions: Arc::clone(&state.transport.web_sessions),
            merge_queue: Arc::clone(&state.transport.merge_queue),
            harness: Arc::clone(&state.execution.harness),
            execution_setup: Arc::clone(&state.execution.setup),
            active_snapshot: Arc::clone(&state.workspaces.workspace_active_snapshot),
            workspace_active_snapshot_cache: Arc::clone(
                &state.workspaces.workspace_active_snapshot_cache,
            ),
            workspace_active_heads_cache: Arc::clone(
                &state.workspaces.workspace_active_heads_cache,
            ),
            workspace_file_completions_cache: Arc::clone(
                &state.workspaces.workspace_file_completions_cache,
            ),
            worktree_file_completions_cache: Arc::clone(&state.workspaces.file_completions_cache),
            worktree_bootstrap_gates: Arc::clone(&state.workspaces.worktree_bootstrap_gates),
            attachment_materialization: Arc::clone(&state.workspaces.attachment_materialization),
            workspace_stores: workspace_stores.clone(),
            session_stores,
            weak_session_stores,
            merge_queue_host,
            task_publication: Arc::clone(&state.task_publication),
            worktree_vcs_runtime: WorktreeVcsRuntimeHost::from_workspace_runtime(&state.workspaces),
            worktree_vcs_execution: WorktreeVcsExecutionHost::new(
                state.core.data_root.clone(),
                state.core.daemon_url.clone(),
                global_store,
                workspace_stores,
                Arc::clone(&state.execution.harness),
            ),
            workspace_vcs_stream_runtime:
                crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime::from_workspace_runtime(
                    &state.workspaces,
                ),
        }
    }
}
