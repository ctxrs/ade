use super::*;

pub struct CoreState {
    pub data_root: PathBuf,
    pub storage_guard: crate::storage_guard::StorageGuardRuntime,
    pub tool_output_spool_enabled: bool,
    pub tool_output_spool_dir: PathBuf,
    pub stores: StoreManager,
    pub daemon_url: String,
    pub auth_token: Option<String>,
    pub lsp_cfg: LspManagerConfig,
    pub lsp: Arc<LspManager>,
    pub lsp_edit_plans_enabled: bool,
    pub buffers: BufferStore,
    pub ask_user_question: Arc<AskUserQuestionBroker>,
    pub shutdown_tx: broadcast::Sender<()>,
}

pub struct SessionRuntime {
    pub session_head_cache:
        Mutex<HashMap<SessionId, TimedEntry<HashMap<SessionHeadCacheKey, SessionHeadSnapshot>>>>,
    pub schedulers: Mutex<HashMap<SessionId, TimedEntry<mpsc::Sender<SchedulerCommand>>>>,
    pub broadcasters: Mutex<HashMap<SessionId, TimedEntry<broadcast::Sender<SessionEvent>>>>,
    pub session_event_heads: Mutex<HashMap<SessionId, TimedEntry<watch::Sender<i64>>>>,
    pub order_seq_states: Mutex<HashMap<SessionId, TimedEntry<Arc<Mutex<OrderSeqState>>>>>,
    pub(crate) active_task_refreshes: Mutex<HashMap<TaskId, ActiveTaskRefreshEntry>>,
    pub running_sessions: Arc<Mutex<HashSet<SessionId>>>,
    pub session_meta_cache: Mutex<HashMap<SessionId, TimedEntry<Session>>>,
}

pub struct WorkspaceRuntime {
    pub file_completions_cache: Mutex<HashMap<WorktreeId, TimedEntry<CachedFileCompletions>>>,
    pub workspace_file_completions_cache:
        Mutex<HashMap<WorkspaceId, TimedEntry<CachedFileCompletions>>>,
    pub git_status_snapshots: Mutex<HashMap<WorktreeId, TimedEntry<GitStatusSnapshotCacheEntry>>>,
    pub worktree_vcs_snapshots:
        Mutex<HashMap<WorktreeId, TimedEntry<WorktreeVcsSnapshotCacheEntry>>>,
    pub worktree_vcs_active: Mutex<HashMap<WorktreeId, usize>>,
    pub worktree_vcs_summary_gen: Mutex<HashMap<WorktreeId, u64>>,
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
    pub edit_plans: Mutex<HashMap<EditPlanId, EditPlan>>,
}

pub struct ProviderRuntime {
    pub adapters: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub target_adapters: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub matrix_cache: Mutex<crate::provider_matrix::ProviderMatrixCache>,
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
    pub claude_login_inputs: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
    pub install_start_gate: Mutex<()>,
    pub installs: Mutex<HashMap<InstallId, InstallState>>,
}

pub struct TelemetryRuntime {
    pub telemetry: Telemetry,
    pub ops_events: OpsEvents,
    pub perf_telemetry: PerfTelemetry,
    pub resource_governance: Mutex<ResourceGovernanceRuntime>,
    pub resource_sampler: Mutex<ResourceSampler>,
}

pub struct TransportRuntime {
    pub terminals: Arc<TerminalManager>,
    pub mobile_tunnel: MobileTunnelManager,
    pub web_sessions: Arc<WebSessionManager>,
    pub merge_queue_notify: Arc<Notify>,
    pub merge_queue_schedule_tx: mpsc::UnboundedSender<WorkspaceId>,
    pub merge_queue_schedule_rx: Mutex<Option<mpsc::UnboundedReceiver<WorkspaceId>>>,
    pub merge_queue_state: Mutex<MergeQueueScheduleState>,
    pub lsp_diag_broadcaster: broadcast::Sender<serde_json::Value>,
    pub lsp_diag_forwarders: Mutex<HashSet<String>>,
}

#[derive(Default)]
pub struct MergeQueueScheduleState {
    pub running: HashSet<WorkspaceId>,
    pub pending: HashSet<WorkspaceId>,
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

pub(crate) struct WorktreeBootstrapGate {
    pub(crate) wait_for_completion: bool,
    pub(crate) done_tx: watch::Sender<bool>,
}

pub(crate) struct AttachmentMaterializationTask {
    pub(crate) generation: u64,
    pub(crate) handle: JoinHandle<()>,
}

pub struct CachedProviderOptions {
    pub cached_at: Instant,
    pub value: serde_json::Value,
}

pub struct CachedProviderVerify {
    pub cached_at: Instant,
    pub value: serde_json::Value,
}

pub struct CachedFileCompletions {
    pub cached_at: Instant,
    pub files: Arc<Vec<String>>,
}

pub struct GitStatusSnapshotCacheEntry {
    pub payload: String,
    pub emitted_at: Instant,
    pub last_change_at: Instant,
}

pub struct WorktreeVcsSnapshotCacheEntry {
    pub snapshot: WorktreeVcsSnapshot,
    pub fingerprint: String,
    pub emitted_at: Instant,
    pub last_change_at: Instant,
    pub last_summary_at: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceActiveSnapshotCacheEntry {
    pub snapshot: WorkspaceActiveSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionHeadCacheKey {
    pub limit: u32,
    pub include_events: bool,
}

#[derive(Clone, Debug)]
pub struct WorkspaceActiveHeadCacheEntry {
    pub batch: WorkspaceActiveHeadBatch,
}

pub(crate) struct ActiveTaskRefreshEntry {
    pub(crate) generation: u64,
}

const DEFAULT_SESSION_CACHE_TTL_HOURS: u64 = 24;
const DEFAULT_WORKSPACE_CACHE_TTL_DAYS: u64 = 1;
const DEFAULT_CACHE_SWEEP_INTERVAL_SECS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug)]
pub struct CacheSweepConfig {
    pub session_ttl: Duration,
    pub workspace_ttl: Duration,
    pub interval: Duration,
}

impl CacheSweepConfig {
    pub fn from_env() -> Self {
        let session_ttl_hours = std::env::var("CTX_SESSION_CACHE_TTL_HOURS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_SESSION_CACHE_TTL_HOURS);
        let workspace_ttl_days = std::env::var("CTX_WORKSPACE_CACHE_TTL_DAYS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_WORKSPACE_CACHE_TTL_DAYS);
        let interval_secs = std::env::var("CTX_CACHE_SWEEP_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_CACHE_SWEEP_INTERVAL_SECS);
        Self {
            session_ttl: Duration::from_secs(session_ttl_hours * 60 * 60),
            workspace_ttl: Duration::from_secs(workspace_ttl_days * 24 * 60 * 60),
            interval: Duration::from_secs(interval_secs.max(30)),
        }
    }
}

#[derive(Default, Debug)]
pub struct CacheSweepStats {
    pub session_head_evicted: usize,
    pub session_meta_evicted: usize,
    pub schedulers_evicted: usize,
    pub broadcasters_evicted: usize,
    pub session_event_heads_evicted: usize,
    pub file_completions_evicted: usize,
    pub workspace_file_completions_evicted: usize,
    pub git_status_evicted: usize,
    pub worktree_vcs_evicted: usize,
    pub workspace_snapshot_evicted: usize,
    pub workspace_heads_evicted: usize,
    pub worktree_bootstrap_evicted: usize,
    pub workspace_stores_evicted: usize,
}

impl CacheSweepStats {
    pub fn total_evicted(&self) -> usize {
        self.session_head_evicted
            + self.session_meta_evicted
            + self.schedulers_evicted
            + self.broadcasters_evicted
            + self.session_event_heads_evicted
            + self.file_completions_evicted
            + self.workspace_file_completions_evicted
            + self.git_status_evicted
            + self.worktree_vcs_evicted
            + self.workspace_snapshot_evicted
            + self.workspace_heads_evicted
            + self.worktree_bootstrap_evicted
            + self.workspace_stores_evicted
    }
}

#[derive(Debug)]
pub struct TimedEntry<T> {
    pub(crate) value: T,
    pub(crate) last_access: Instant,
}

impl<T> TimedEntry<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            value,
            last_access: Instant::now(),
        }
    }

    pub(crate) fn touch(&mut self) {
        self.last_access = Instant::now();
    }

    pub(crate) fn touch_at(&mut self, now: Instant) {
        self.last_access = now;
    }
}
