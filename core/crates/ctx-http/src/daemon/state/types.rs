use super::*;
use ctx_execution_runtime::ExecutionSetupCoordinator;

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
    pub update_drain: Arc<Mutex<Option<UpdateDrainState>>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct UpdateDrainState {
    pub reason: String,
    pub owner: String,
    pub acquired_at_ms: u64,
}

pub struct SessionRuntime {
    pub session_head_cache:
        Mutex<HashMap<SessionId, TimedEntry<HashMap<SessionHeadCacheKey, SessionHeadSnapshot>>>>,
    pub schedulers: Mutex<HashMap<SessionId, TimedEntry<mpsc::Sender<SchedulerCommand>>>>,
    pub provider_inactivity_timeout: Mutex<Duration>,
    pub broadcasters: Mutex<HashMap<SessionId, TimedEntry<broadcast::Sender<SessionEvent>>>>,
    pub session_event_heads: Mutex<HashMap<SessionId, TimedEntry<watch::Sender<i64>>>>,
    pub order_seq_states: Mutex<HashMap<SessionId, TimedEntry<Arc<Mutex<OrderSeqState>>>>>,
    pub(crate) active_task_refreshes: Mutex<HashMap<TaskId, ActiveTaskRefreshEntry>>,
    pub(crate) task_session_creation_locks:
        Mutex<HashMap<TaskId, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    pub running_sessions: Arc<Mutex<HashSet<SessionId>>>,
    pub session_pins: Mutex<HashMap<SessionId, SessionPinState>>,
    pub session_meta_cache: Mutex<HashMap<SessionId, TimedEntry<Session>>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionPinState {
    pub running: bool,
    pub attached_clients: usize,
}

impl SessionPinState {
    pub fn is_pinned(self) -> bool {
        self.running || self.attached_clients > 0
    }
}

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
    pub merge_queue: Arc<ctx_merge_queue::MergeQueueRuntime>,
    pub lsp_diag_broadcaster: broadcast::Sender<serde_json::Value>,
    pub lsp_diag_forwarders: Mutex<HashSet<String>>,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorktreeVcsDirtyBits {
    pub worktree_fs: bool,
    pub vcs_meta: bool,
}

impl WorktreeVcsDirtyBits {
    pub fn any(self) -> bool {
        self.worktree_fs || self.vcs_meta
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorktreeVcsRuntimeState {
    pub generation: u64,
    pub dirty_bits: WorktreeVcsDirtyBits,
    pub pending_summary: bool,
    pub pending_touched_files: bool,
    pub running: bool,
    pub require_full_summary_rebuild: bool,
    pub summary_paths: HashSet<String>,
    pub candidate_paths: BTreeSet<String>,
    pub last_git_status: Option<GitStatusSnapshot>,
    pub touched_files: WorktreeVcsTouchedFiles,
    pub touched_files_state: WorktreeVcsTouchedFilesState,
}

pub struct WorktreeVcsSchedulerRuntime {
    pub started: AtomicBool,
    pub notify: Arc<Notify>,
    pub permits: Arc<Semaphore>,
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
const DEFAULT_PROVIDER_INACTIVITY_TIMEOUT_SECS: u64 = 30 * 60;
const DEFAULT_WORKTREE_VCS_SCHEDULER_CONCURRENCY: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppRuntimeFlags {
    pub lsp_edit_plans_enabled: bool,
    pub worktree_vcs_enabled: bool,
}

fn parse_worktree_vcs_enabled(raw: &str) -> Option<bool> {
    ctx_core::boolish::parse_boolish(raw).or_else(|| {
        match raw.trim().to_ascii_lowercase().as_str() {
            "enabled" => Some(true),
            "disabled" => Some(false),
            _ => None,
        }
    })
}

pub(crate) fn worktree_vcs_enabled_from_env() -> bool {
    for key in ["CTX_WORKTREE_VCS_ENABLED", "CTX_WORKTREE_VCS"] {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        if let Some(parsed) = parse_worktree_vcs_enabled(&value) {
            return parsed;
        }
        tracing::warn!(
            env_var = key,
            value = %value,
            "ignoring invalid worktree VCS mode; expected on/off, true/false, enabled/disabled, or 1/0"
        );
    }
    true
}

pub(crate) fn provider_inactivity_timeout_from_env() -> Duration {
    std::env::var("CTX_PROVIDER_TURN_INACTIVITY_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_PROVIDER_INACTIVITY_TIMEOUT_SECS))
}

pub(crate) fn worktree_vcs_scheduler_concurrency_from_env() -> usize {
    std::env::var("CTX_WORKTREE_VCS_SCHEDULER_CONCURRENCY")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_WORKTREE_VCS_SCHEDULER_CONCURRENCY)
}

#[cfg(test)]
mod tests {
    use super::parse_worktree_vcs_enabled;

    #[test]
    fn parses_worktree_vcs_mode_values() {
        for raw in ["1", "true", "yes", "on", "enabled"] {
            assert_eq!(parse_worktree_vcs_enabled(raw), Some(true), "raw={raw}");
        }
        for raw in ["0", "false", "no", "off", "disabled"] {
            assert_eq!(parse_worktree_vcs_enabled(raw), Some(false), "raw={raw}");
        }
        for raw in ["", "maybe", "summary-only"] {
            assert_eq!(parse_worktree_vcs_enabled(raw), None, "raw={raw}");
        }
    }
}

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
