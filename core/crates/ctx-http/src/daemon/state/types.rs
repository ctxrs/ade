use super::*;
use crate::daemon::McpAuthContext;
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_provider_runtime::{CachedProviderOptions, CachedProviderVerify};
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_active_snapshot::{
    WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
};
use ctx_workspace_services::file_completions::CachedFileCompletions;
use ctx_workspace_services::worktree_vcs::{
    GitStatusSnapshotCacheEntry, WorktreeVcsRuntimeState, WorktreeVcsSchedulerRuntime,
};

pub struct CoreState {
    pub data_root: PathBuf,
    pub storage_guard: crate::storage_guard::StorageGuardRuntime,
    pub tool_output_spool_enabled: bool,
    pub tool_output_spool_dir: PathBuf,
    pub stores: StoreManager,
    pub daemon_url: String,
    pub public_base_url: Option<String>,
    pub auth_token: Option<String>,
    pub local_shutdown_token: Option<String>,
    pub(crate) mcp_auth: Mutex<HashMap<String, TimedEntry<McpAuthContext>>>,
    pub ask_user_question: Arc<AskUserQuestionBroker>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub update_drain: Arc<UpdateDrainCoordinator>,
}

pub type SessionRuntime = ctx_session_service::runtime::SessionRuntime<SchedulerCommand>;

pub use ctx_session_service::runtime::SessionHeadCacheKey;

pub struct WorkspaceRuntime {
    pub worktree_vcs_enabled: bool,
    pub file_completions_cache: Mutex<HashMap<WorktreeId, TimedEntry<CachedFileCompletions>>>,
    pub workspace_file_completions_cache:
        Mutex<HashMap<WorkspaceId, TimedEntry<CachedFileCompletions>>>,
    pub git_status_snapshots: Mutex<HashMap<WorktreeId, TimedEntry<GitStatusSnapshotCacheEntry>>>,
    pub worktree_vcs_snapshots:
        Mutex<HashMap<WorktreeId, TimedEntry<WorktreeVcsSnapshotCacheEntry>>>,
    pub worktree_vcs_active: Mutex<HashMap<WorktreeId, usize>>,
    pub(crate) worktree_vcs_refresh_locks: Mutex<HashMap<WorktreeId, std::sync::Weak<Mutex<()>>>>,
    pub worktree_vcs_open_panes: Mutex<HashMap<WorktreeId, usize>>,
    pub worktree_vcs_summary_gen: Mutex<HashMap<WorktreeId, u64>>,
    pub worktree_vcs_runtime: Mutex<HashMap<WorktreeId, WorktreeVcsRuntimeState>>,
    pub worktree_vcs_scheduler: WorktreeVcsSchedulerRuntime,
    pub git_status_watchers: Mutex<HashSet<WorktreeId>>,
    pub workspace_active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    pub workspace_active_snapshot_cache:
        Mutex<HashMap<WorkspaceId, TimedEntry<WorkspaceActiveSnapshotCacheEntry>>>,
    pub workspace_active_heads_cache:
        Mutex<HashMap<WorkspaceId, TimedEntry<WorkspaceActiveHeadCacheEntry>>>,
    pub(crate) worktree_bootstrap_gates:
        Mutex<HashMap<WorktreeId, TimedEntry<WorktreeBootstrapGate>>>,
    pub(crate) attachment_materializations:
        Mutex<HashMap<WorkspaceAttachmentId, AttachmentMaterializationTask>>,
    pub(crate) attachment_materialization_generation: AtomicU64,
}

pub struct ProviderRuntime {
    pub adapters: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub target_adapters: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub matrix_cache: Mutex<ctx_provider_matrix::ProviderMatrixCache>,
    pub options_cache: Mutex<HashMap<String, CachedProviderOptions>>,
    pub verify_cache: Mutex<HashMap<String, CachedProviderVerify>>,
    pub guard: Mutex<provider_guard::ProviderGuardRuntime>,
    pub restart: Mutex<provider_restart::ProviderRestartRuntime>,
    pub usage_cache: Mutex<HashMap<String, provider_usage::ProviderUsageSnapshot>>,
    pub codex_login_sessions: Mutex<HashMap<String, provider_accounts::CodexLoginStatus>>,
    pub claude_login_sessions: Mutex<HashMap<String, provider_accounts::ClaudeLoginStatus>>,
    pub gemini_login_sessions: Mutex<HashMap<String, provider_accounts::GeminiLoginStatus>>,
    pub qwen_login_sessions: Mutex<HashMap<String, provider_accounts::QwenLoginStatus>>,
    pub kimi_login_sessions: Mutex<HashMap<String, provider_accounts::KimiLoginStatus>>,
    pub cursor_login_sessions: Mutex<HashMap<String, provider_accounts::CursorLoginStatus>>,
    pub amp_login_sessions: Mutex<HashMap<String, provider_accounts::AmpLoginStatus>>,
    pub mistral_login_sessions: Mutex<HashMap<String, provider_accounts::MistralLoginStatus>>,
    pub install_start_gate: Mutex<()>,
    pub installs: Mutex<HashMap<InstallId, InstallState>>,
}

pub struct TelemetryRuntime {
    pub telemetry: Telemetry,
    pub ops_events: OpsEvents,
    pub perf_telemetry: PerfTelemetry,
    pub(crate) provider_unknown_events:
        ctx_observability::provider_unknown_events::ProviderUnknownEvents,
    pub resource_governance: Mutex<ResourceGovernanceRuntime>,
    pub resource_sampler: Mutex<ResourceSampler>,
}

pub struct TransportRuntime {
    pub terminals: Arc<TerminalManager>,
    pub mobile_tunnel: MobileTunnelManager,
    pub web_sessions: Arc<WebSessionManager>,
    pub merge_queue: Arc<ctx_merge_queue::MergeQueueRuntime>,
}

pub struct ExecutionRuntime {
    pub harness: Arc<HarnessRuntimeManager>,
    pub setup: Arc<ExecutionSetupCoordinator>,
}

pub struct AppState {
    pub core: CoreState,
    pub sessions: SessionRuntime,
    pub workspaces: WorkspaceRuntime,
    pub providers: ProviderRuntime,
    pub telemetry: TelemetryRuntime,
    pub transport: TransportRuntime,
    pub execution: ExecutionRuntime,
}

pub enum StoreLookup {
    Found(Store),
    Missing,
    Deleting,
    Unavailable(anyhow::Error),
}

pub(crate) struct WorktreeBootstrapGate {
    pub(crate) wait_for_completion: bool,
    pub(crate) done_tx: watch::Sender<bool>,
}

pub(crate) struct AttachmentMaterializationTask {
    pub(crate) generation: u64,
    pub(crate) handle: JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppRuntimeFlags {
    pub worktree_vcs_enabled: bool,
}
