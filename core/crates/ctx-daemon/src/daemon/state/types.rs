use super::*;
use ctx_core::models::WorktreeVcsSnapshot;
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_mcp_auth::McpAuthRegistry;
use ctx_storage_admission::StorageGuardRuntime;
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_active_snapshot::{
    WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
};
use ctx_worktree_vcs_service::CachedFileCompletions;
use ctx_worktree_vcs_service::{
    GitStatusSnapshotCacheEntry, WorktreeVcsRuntimeState, WorktreeVcsSchedulerRuntime,
    WorktreeVcsSnapshotCacheEntry,
};

pub struct CoreState {
    pub(crate) data_root: PathBuf,
    pub(crate) storage_guard: Arc<StorageGuardRuntime>,
    pub(crate) tool_output_spool_enabled: bool,
    pub(crate) tool_output_spool_dir: PathBuf,
    pub(crate) stores: StoreManager,
    pub(crate) daemon_url: String,
    pub(crate) public_base_url: Option<String>,
    pub(crate) auth_token: Option<String>,
    pub(crate) local_shutdown_token: Option<String>,
    pub(crate) mcp_auth: Arc<McpAuthRegistry>,
    pub(crate) ask_user_question: Arc<AskUserQuestionBroker>,
    pub(crate) shutdown_tx: broadcast::Sender<()>,
    pub(crate) update_drain: Arc<UpdateDrainCoordinator>,
}

pub type SessionRuntime = ctx_session_runtime::runtime::SessionRuntime<SchedulerCommand>;

pub struct WorkspaceRuntime {
    pub(crate) worktree_vcs_enabled: bool,
    pub(crate) file_completions_cache:
        Mutex<HashMap<WorktreeId, TimedEntry<CachedFileCompletions>>>,
    pub(crate) workspace_file_completions_cache:
        Mutex<HashMap<WorkspaceId, TimedEntry<CachedFileCompletions>>>,
    pub(crate) git_status_snapshots:
        Mutex<HashMap<WorktreeId, TimedEntry<GitStatusSnapshotCacheEntry>>>,
    pub(crate) worktree_vcs_snapshots:
        Mutex<HashMap<WorktreeId, TimedEntry<WorktreeVcsSnapshotCacheEntry>>>,
    pub(crate) worktree_vcs_active: Mutex<HashMap<WorktreeId, usize>>,
    pub(crate) worktree_vcs_refresh_locks: Mutex<HashMap<WorktreeId, std::sync::Weak<Mutex<()>>>>,
    pub(crate) worktree_vcs_open_panes: Mutex<HashMap<WorktreeId, usize>>,
    pub(crate) worktree_vcs_summary_gen: Mutex<HashMap<WorktreeId, u64>>,
    pub(crate) worktree_vcs_runtime: Mutex<HashMap<WorktreeId, WorktreeVcsRuntimeState>>,
    pub(crate) worktree_vcs_scheduler: WorktreeVcsSchedulerRuntime,
    pub(crate) worktree_vcs_events: broadcast::Sender<WorktreeVcsSnapshot>,
    pub(crate) git_status_watchers: Mutex<HashSet<WorktreeId>>,
    pub(crate) workspace_active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    pub(crate) workspace_active_snapshot_cache:
        Mutex<HashMap<WorkspaceId, TimedEntry<WorkspaceActiveSnapshotCacheEntry>>>,
    pub(crate) workspace_active_heads_cache:
        Mutex<HashMap<WorkspaceId, TimedEntry<WorkspaceActiveHeadCacheEntry>>>,
    pub(crate) worktree_bootstrap_gates:
        Mutex<HashMap<WorktreeId, TimedEntry<WorktreeBootstrapGate>>>,
    pub(crate) attachment_materializations:
        Mutex<HashMap<WorkspaceAttachmentId, AttachmentMaterializationTask>>,
    pub(crate) attachment_materialization_generation: AtomicU64,
}

pub type ProviderRuntime = ctx_provider_runtime::ProviderRuntime;

pub struct TelemetryRuntime {
    pub(crate) telemetry: Telemetry,
    pub(crate) ops_events: OpsEvents,
    pub(crate) perf_telemetry: PerfTelemetry,
    pub(crate) provider_unknown_events:
        ctx_observability::provider_unknown_events::ProviderUnknownEvents,
    pub(crate) resource_governance: Mutex<ResourceGovernanceRuntime>,
    pub(crate) resource_sampler: Mutex<ResourceSampler>,
}

pub struct TransportRuntime {
    pub(crate) terminals: Arc<TerminalManager>,
    pub(crate) mobile_tunnel: MobileTunnelManager,
    pub(crate) web_sessions: Arc<WebSessionManager>,
    pub(crate) merge_queue: Arc<ctx_merge_queue::MergeQueueRuntime>,
}

pub struct ExecutionRuntime {
    pub(crate) harness: Arc<HarnessRuntimeManager>,
    pub(crate) setup: Arc<ExecutionSetupCoordinator>,
}

pub struct DaemonState {
    pub(crate) core: CoreState,
    pub(crate) sessions: SessionRuntime,
    pub(crate) workspaces: WorkspaceRuntime,
    pub(crate) providers: ProviderRuntime,
    pub(crate) telemetry: TelemetryRuntime,
    pub(crate) transport: TransportRuntime,
    pub(crate) execution: ExecutionRuntime,
}

pub enum StoreLookup {
    Found(Store),
    Missing,
    Deleting,
    Unavailable(anyhow::Error),
}

pub struct WorktreeBootstrapGate {
    pub wait_for_completion: bool,
    pub done_tx: watch::Sender<bool>,
}

pub struct AttachmentMaterializationTask {
    pub generation: u64,
    pub handle: JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppRuntimeFlags {
    pub worktree_vcs_enabled: bool,
}
