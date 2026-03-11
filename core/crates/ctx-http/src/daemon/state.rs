use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, watch, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::buffers::BufferStore;
use crate::edit_plans::{EditPlan, EditPlanId};
use crate::execution_setup::ExecutionSetupCoordinator;
use crate::harness_runtime::HarnessRuntimeManager;
use crate::installs::{
    InstallErrorCode, InstallEventLevel, InstallId, InstallProgressEvent, InstallState,
    InstallStateKind, InstallTarget,
};
use crate::mobile_tunnel::MobileTunnelManager;
use crate::ops_events::{OpsEvent, OpsEvents};
use crate::order_seq::OrderSeqState;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use crate::provider_accounts;
use crate::provider_guard;
use crate::provider_restart;
use crate::provider_usage;
use crate::resource_governance::ResourceGovernanceRuntime;
use crate::resource_utilization::ResourceSampler;
use crate::scheduler::SchedulerCommand;
use crate::telemetry::Telemetry;
use crate::terminals::TerminalManager;
use crate::web_sessions::WebSessionManager;
use crate::workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_core::ids::{
    ArtifactId, MergeQueueEntryId, MessageId, SessionId, TaskId, WorkspaceAttachmentId,
    WorkspaceId, WorktreeId,
};
use ctx_core::models::{
    Session, SessionEvent, SessionHeadSnapshot, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot,
    WorktreeVcsSnapshot,
};
use ctx_lsp::{LspManager, LspManagerConfig};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use ctx_providers::ask_user_question::AskUserQuestionBroker;
use ctx_store::{Store, StoreManager};

use super::edit_plans;
pub struct CoreState {
    pub data_root: PathBuf,
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
    pub(crate) active_head_projections: Mutex<HashMap<SessionId, ActiveHeadProjectionEntry>>,
    pub(crate) active_task_refreshes: Mutex<HashMap<TaskId, ActiveTaskRefreshEntry>>,
    pub running_sessions: Mutex<HashSet<SessionId>>,
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
    pub claude_oauth_login_sessions:
        Mutex<HashMap<String, provider_accounts::ClaudeOauthLoginSession>>,
    pub gemini_login_sessions: Mutex<HashMap<String, provider_accounts::GeminiLoginStatus>>,
    pub qwen_login_sessions: Mutex<HashMap<String, provider_accounts::QwenLoginStatus>>,
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

#[derive(Clone, Debug)]
pub(crate) struct ActiveHeadProjectionEntry {
    pub(crate) last_event_seq: i64,
    pub(crate) last_event_at: Instant,
    pub(crate) last_flushed_seq: i64,
    pub(crate) last_flush_at: Instant,
}

pub(crate) struct ActiveTaskRefreshEntry {
    pub(crate) generation: u64,
}

const DEFAULT_SESSION_CACHE_TTL_HOURS: u64 = 24;
const DEFAULT_WORKSPACE_CACHE_TTL_DAYS: u64 = 7;
const DEFAULT_CACHE_SWEEP_INTERVAL_SECS: u64 = 60 * 60;
const INSTALL_TIMEOUT_GRACE_SECS: u64 = 90;
const INSTALL_TIMEOUT_VENV_SECS: u64 = (5 * 60) + INSTALL_TIMEOUT_GRACE_SECS;
const INSTALL_TIMEOUT_DOWNLOAD_SECS: u64 = (15 * 60) + INSTALL_TIMEOUT_GRACE_SECS;
const INSTALL_TIMEOUT_PACKAGE_MANAGER_SECS: u64 = (12 * 60) + INSTALL_TIMEOUT_GRACE_SECS;
const INSTALL_TIMEOUT_PREPARE_SECS: u64 = 5 * 60;
const INSTALL_TIMEOUT_REGISTRY_SECS: u64 = 2 * 60;
const INSTALL_TIMEOUT_DEFAULT_SECS: u64 = 20 * 60;
const PREREQUISITE_PROGRESS_STAGE_FLOOR: &str = "start";
const PREREQUISITE_PROGRESS_VISIBILITY_MS: i64 = 1_200;

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

impl AppState {
    fn install_running_timeout_for_stage(stage: &str) -> Duration {
        match stage {
            "download" | "node_download" | "python_download" | "model_download"
            | "runtime_download" => Duration::from_secs(INSTALL_TIMEOUT_DOWNLOAD_SECS),
            "npm_install" | "dependency_npm_install" | "pip_install" => {
                Duration::from_secs(INSTALL_TIMEOUT_PACKAGE_MANAGER_SECS)
            }
            "venv" => Duration::from_secs(INSTALL_TIMEOUT_VENV_SECS),
            "registry" | "registry_load" | "registry_save" => {
                Duration::from_secs(INSTALL_TIMEOUT_REGISTRY_SECS)
            }
            "prepare" | "extract" | "node_extract" | "python_extract" | "runtime_extract" => {
                Duration::from_secs(INSTALL_TIMEOUT_PREPARE_SECS)
            }
            _ => Duration::from_secs(INSTALL_TIMEOUT_DEFAULT_SECS),
        }
    }

    fn format_install_timeout(duration: Duration) -> String {
        let secs = duration.as_secs();
        if secs >= 60 {
            let mins = secs / 60;
            let rem = secs % 60;
            if rem == 0 {
                format!("{mins}m")
            } else {
                format!("{mins}m {rem}s")
            }
        } else {
            format!("{secs}s")
        }
    }

    fn reconcile_stale_running_install_locked(
        &self,
        install_id: InstallId,
        st: &mut InstallState,
    ) -> bool {
        if !matches!(st.state, InstallStateKind::Running) {
            return false;
        }
        let now = chrono::Utc::now();
        let last_event = st.events.back().cloned();
        let stage = last_event
            .as_ref()
            .map(|event| event.stage.trim())
            .filter(|stage| !stage.is_empty())
            .unwrap_or("prepare");
        let anchor = last_event
            .as_ref()
            .map(|event| event.at)
            .unwrap_or(st.started_at);
        let Ok(inactive_for) = now.signed_duration_since(anchor).to_std() else {
            return false;
        };
        let timeout_after = Self::install_running_timeout_for_stage(stage);
        if inactive_for <= timeout_after {
            return false;
        }

        let message = format!(
            "Install timed out during {stage} after {} without progress. Retry the install.",
            Self::format_install_timeout(inactive_for)
        );
        st.state = InstallStateKind::Failed;
        st.finished_at = Some(now);
        st.error = Some(message.clone());
        st.error_code = Some(InstallErrorCode::Timeout);
        let event = InstallProgressEvent {
            install_id,
            provider_id: st.provider_id.clone(),
            target: st.target,
            at: now,
            stage: stage.to_string(),
            message: message.clone(),
            level: InstallEventLevel::Error,
            bytes: None,
            total_bytes: None,
            attempt: None,
            error_code: Some(InstallErrorCode::Timeout),
        };
        if st.events.len() >= 256 {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        let _ = st.tx.send(event);

        let mut ops_event = OpsEvent::new("warn", "provider_install_failed");
        ops_event.provider_id = Some(st.provider_id.clone());
        ops_event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": st.target.map(|value| value.as_str()),
            "state": "failed",
            "error": message,
            "error_code": "timeout",
            "ok": false,
        }));
        self.telemetry.ops_events.emit(ops_event);
        true
    }

    pub fn new(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
    ) -> Self {
        Self::new_with_lsp_config(
            data_root,
            stores,
            providers,
            daemon_url,
            auth_token,
            LspManagerConfig::default(),
        )
    }

    pub fn new_with_lsp_config(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
        lsp_cfg: LspManagerConfig,
    ) -> Self {
        let lsp_edit_plans_enabled = std::env::var("CTX_LSP_EDITPLANS_ENABLED")
            .ok()
            .as_deref()
            .and_then(ctx_core::boolish::parse_boolish)
            .unwrap_or(false);
        Self::new_with_lsp_config_and_flags(
            data_root,
            stores,
            providers,
            daemon_url,
            auth_token,
            lsp_cfg,
            lsp_edit_plans_enabled,
        )
    }

    pub fn new_with_lsp_config_and_flags(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
        lsp_cfg: LspManagerConfig,
        lsp_edit_plans_enabled: bool,
    ) -> Self {
        // Internal/experimental only: tool output disk spooling is not a supported
        // v1 product surface and must not be treated as a stable client contract.
        let tool_output_spool_enabled = std::env::var("CTX_TOOL_OUTPUT_DISK_SPOOL")
            .ok()
            .as_deref()
            .and_then(ctx_core::boolish::parse_boolish)
            .unwrap_or(false);
        let tool_output_spool_dir = data_root.join("tool-output-spool");
        if tool_output_spool_enabled {
            if let Err(e) = std::fs::create_dir_all(&tool_output_spool_dir) {
                tracing::warn!(
                    "failed to create tool output spool dir {}: {e}",
                    tool_output_spool_dir.to_string_lossy()
                );
            }
        }
        let edit_plans_dir = edit_plans::edit_plans_dir(&data_root);
        if let Err(e) = std::fs::create_dir_all(&edit_plans_dir) {
            tracing::warn!(
                "failed to create edit plans dir {}: {e}",
                edit_plans_dir.to_string_lossy()
            );
        }
        let edit_plans = edit_plans::load_edit_plans_from_disk(&data_root);

        let (shutdown_tx, _) = broadcast::channel(8);
        let (lsp_diag_broadcaster, _) = broadcast::channel(2048);
        let ask_user_question = Arc::new(AskUserQuestionBroker::new());
        let lsp = Arc::new(LspManager::new(lsp_cfg.clone()));
        let telemetry = Telemetry::new(data_root.clone());
        let ops_events = OpsEvents::new(data_root.clone());
        let perf_telemetry = PerfTelemetry::new(data_root.clone());
        let harness_runtime = Arc::new(HarnessRuntimeManager::new(data_root.clone()));
        let execution_setup = Arc::new(ExecutionSetupCoordinator::new(
            data_root.clone(),
            harness_runtime.clone(),
            perf_telemetry.clone(),
            ops_events.clone(),
        ));
        execution_setup.spawn_startup_prewarm();
        let workspace_active_snapshot = Arc::new(WorkspaceActiveSnapshotHub::new());
        let web_sessions = Arc::new(WebSessionManager::new());
        let merge_queue_notify = Arc::new(Notify::new());

        Self {
            core: CoreState {
                data_root,
                tool_output_spool_enabled,
                tool_output_spool_dir,
                stores,
                daemon_url,
                auth_token,
                lsp_cfg,
                lsp,
                lsp_edit_plans_enabled,
                buffers: BufferStore::default(),
                ask_user_question,
                shutdown_tx,
            },
            sessions: SessionRuntime {
                session_head_cache: Mutex::new(HashMap::new()),
                schedulers: Mutex::new(HashMap::new()),
                broadcasters: Mutex::new(HashMap::new()),
                session_event_heads: Mutex::new(HashMap::new()),
                order_seq_states: Mutex::new(HashMap::new()),
                active_head_projections: Mutex::new(HashMap::new()),
                active_task_refreshes: Mutex::new(HashMap::new()),
                running_sessions: Mutex::new(HashSet::new()),
                session_meta_cache: Mutex::new(HashMap::new()),
            },
            workspaces: WorkspaceRuntime {
                file_completions_cache: Mutex::new(HashMap::new()),
                workspace_file_completions_cache: Mutex::new(HashMap::new()),
                git_status_snapshots: Mutex::new(HashMap::new()),
                worktree_vcs_snapshots: Mutex::new(HashMap::new()),
                worktree_vcs_active: Mutex::new(HashMap::new()),
                worktree_vcs_summary_gen: Mutex::new(HashMap::new()),
                git_status_watchers: Mutex::new(HashSet::new()),
                workspace_active_snapshot,
                workspace_active_snapshot_cache: Mutex::new(HashMap::new()),
                workspace_active_heads_cache: Mutex::new(HashMap::new()),
                worktree_bootstrap_gates: Mutex::new(HashMap::new()),
                attachment_materializations: Mutex::new(HashMap::new()),
                attachment_materialization_generation: AtomicU64::new(0),
                edit_plans: Mutex::new(edit_plans),
            },
            providers: ProviderRuntime {
                adapters: Mutex::new(providers),
                target_adapters: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
                matrix_cache: Mutex::new(crate::provider_matrix::ProviderMatrixCache::default()),
                options_cache: Mutex::new(HashMap::new()),
                verify_cache: Mutex::new(HashMap::new()),
                guard: Mutex::new(provider_guard::ProviderGuardRuntime::default()),
                restart: Mutex::new(provider_restart::ProviderRestartRuntime::default()),
                usage_cache: Mutex::new(HashMap::new()),
                codex_login_sessions: Mutex::new(HashMap::new()),
                claude_login_sessions: Mutex::new(HashMap::new()),
                claude_oauth_login_sessions: Mutex::new(HashMap::new()),
                gemini_login_sessions: Mutex::new(HashMap::new()),
                qwen_login_sessions: Mutex::new(HashMap::new()),
                amp_login_sessions: Mutex::new(HashMap::new()),
                mistral_login_sessions: Mutex::new(HashMap::new()),
                claude_login_inputs: Mutex::new(HashMap::new()),
                install_start_gate: Mutex::new(()),
                installs: Mutex::new(HashMap::new()),
            },
            telemetry: TelemetryRuntime {
                telemetry,
                ops_events,
                perf_telemetry,
                resource_governance: Mutex::new(ResourceGovernanceRuntime::default()),
                resource_sampler: Mutex::new(ResourceSampler::new()),
            },
            transport: TransportRuntime {
                terminals: Arc::new(TerminalManager::default()),
                mobile_tunnel: MobileTunnelManager::default(),
                web_sessions,
                merge_queue_notify,
                lsp_diag_broadcaster,
                lsp_diag_forwarders: Mutex::new(HashSet::new()),
            },
            execution: ExecutionRuntime {
                harness: harness_runtime,
                setup: execution_setup,
            },
        }
    }

    pub fn edit_plans_dir(&self) -> PathBuf {
        edit_plans::edit_plans_dir(&self.core.data_root)
    }

    pub fn persist_edit_plan(&self, plan: &EditPlan) {
        if let Err(e) = edit_plans::persist_edit_plan_to_disk(&self.core.data_root, plan) {
            tracing::warn!("failed to persist edit plan {}: {e}", plan.id.0);
        }
    }

    pub fn delete_edit_plan_file(&self, plan_id: EditPlanId) {
        if let Err(e) = edit_plans::delete_edit_plan_file(&self.core.data_root, plan_id) {
            tracing::warn!("failed to delete edit plan {} file: {e}", plan_id.0);
        }
    }

    pub fn global_store(&self) -> &Store {
        self.core.stores.global()
    }

    pub async fn store_for_workspace(&self, workspace_id: WorkspaceId) -> Result<Store> {
        self.core.stores.workspace(workspace_id).await
    }

    pub async fn store_for_task(&self, task_id: TaskId) -> Result<Store> {
        self.core.stores.store_for_task(task_id).await
    }

    pub async fn store_for_session(&self, session_id: SessionId) -> Result<Store> {
        self.core.stores.store_for_session(session_id).await
    }

    pub async fn store_for_worktree(&self, worktree_id: WorktreeId) -> Result<Store> {
        self.core.stores.store_for_worktree(worktree_id).await
    }

    pub async fn store_for_artifact(&self, artifact_id: ArtifactId) -> Result<Store> {
        self.core.stores.store_for_artifact(artifact_id).await
    }

    pub async fn store_for_message(&self, message_id: MessageId) -> Result<Store> {
        self.core.stores.store_for_message(message_id).await
    }

    pub async fn store_for_subagent_invocation(&self, invocation_id: &str) -> Result<Store> {
        self.core
            .stores
            .store_for_subagent_invocation(invocation_id)
            .await
    }

    pub async fn store_for_merge_queue_entry(&self, entry_id: MergeQueueEntryId) -> Result<Store> {
        self.core.stores.store_for_merge_queue_entry(entry_id).await
    }

    pub fn lsp_diag_broadcaster(&self) -> broadcast::Sender<serde_json::Value> {
        self.transport.lsp_diag_broadcaster.clone()
    }

    pub(crate) async fn emit_cache_miss(&self, cache: &str) {
        self.emit_cache_counter("daemon.cache_miss", cache, 1, None)
            .await;
    }

    pub(crate) async fn emit_cache_rehydrate(&self, cache: &str, ok: bool) {
        let result = if ok { "ok" } else { "fail" };
        self.emit_cache_counter("daemon.cache_rehydrate", cache, 1, Some(("result", result)))
            .await;
    }

    async fn emit_cache_evicted(&self, cache: &str, value: usize) {
        if value == 0 {
            return;
        }
        self.emit_cache_counter("daemon.cache_evicted", cache, value as u64, None)
            .await;
    }

    async fn emit_cache_counter(
        &self,
        name: &str,
        cache: &str,
        value: u64,
        extra_label: Option<(&str, &str)>,
    ) {
        if value == 0 {
            return;
        }
        let mut labels = HashMap::new();
        labels.insert("cache".to_string(), cache.to_string());
        labels.insert("source".to_string(), "daemon".to_string());
        if let Some((key, val)) = extra_label {
            labels.insert(key.to_string(), val.to_string());
        }
        let metric = PerfMetric {
            name: name.to_string(),
            kind: PerfMetricKind::Counter,
            unit: "count".to_string(),
            value: value as f64,
            labels,
        };
        self.telemetry
            .perf_telemetry
            .record_metric(metric, None, None, None)
            .await;
    }

    pub(crate) async fn emit_compat_payload_reject_counter(
        &self,
        surface: &str,
        issue: &str,
        extra_label: Option<(&str, &str)>,
    ) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("surface".to_string(), surface.to_string());
        labels.insert("issue".to_string(), issue.to_string());
        if let Some((key, value)) = extra_label {
            labels.insert(key.to_string(), value.to_string());
        }
        self.emit_counter_metric("compat.payload_reject_count", labels)
            .await;
    }

    pub(crate) async fn emit_product_fallback_applied_counter(
        &self,
        surface: &str,
        fallback: &str,
        extra_label: Option<(&str, &str)>,
    ) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("surface".to_string(), surface.to_string());
        labels.insert("fallback".to_string(), fallback.to_string());
        if let Some((key, value)) = extra_label {
            labels.insert(key.to_string(), value.to_string());
        }
        self.emit_counter_metric("product.fallback_applied_count", labels)
            .await;
    }

    async fn emit_counter_metric(&self, name: &str, labels: HashMap<String, String>) {
        let metric = PerfMetric {
            name: name.to_string(),
            kind: PerfMetricKind::Counter,
            unit: "count".to_string(),
            value: 1.0,
            labels,
        };
        self.telemetry
            .perf_telemetry
            .record_metric(metric, None, None, None)
            .await;
    }

    pub async fn sweep_idle_caches(
        &self,
        now: Instant,
        config: CacheSweepConfig,
    ) -> CacheSweepStats {
        let mut stats = CacheSweepStats::default();
        let running_sessions = {
            let set = self.sessions.running_sessions.lock().await;
            set.iter().copied().collect::<HashSet<_>>()
        };
        {
            let mut cache = self.sessions.session_head_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_head_evicted += expired.len();
        }
        {
            let mut cache = self.sessions.session_meta_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_meta_evicted += expired.len();
        }
        {
            let mut map = self.sessions.schedulers.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if entry.value.is_closed()
                        || now.duration_since(entry.last_access) >= config.session_ttl
                    {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.schedulers_evicted += expired.len();
        }
        {
            let mut map = self.sessions.broadcasters.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.broadcasters_evicted += expired.len();
        }
        {
            let mut map = self.sessions.session_event_heads.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.session_event_heads_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.file_completions_cache.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.file_completions_evicted += expired.len();
        }
        {
            let mut cache = self
                .workspaces
                .workspace_file_completions_cache
                .lock()
                .await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_file_completions_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.git_status_snapshots.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.git_status_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.worktree_vcs_snapshots.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.worktree_vcs_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.workspace_active_snapshot_cache.lock().await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_snapshot_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.workspace_active_heads_cache.lock().await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_heads_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.worktree_bootstrap_gates.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.worktree_bootstrap_evicted += expired.len();
        }
        let mut active_sessions: HashSet<SessionId> = HashSet::new();
        {
            let set = self.sessions.running_sessions.lock().await;
            active_sessions.extend(set.iter().copied());
        }
        {
            let map = self.sessions.schedulers.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.sessions.broadcasters.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.sessions.session_event_heads.lock().await;
            active_sessions.extend(map.keys().copied());
        }

        let mut active_workspaces: HashSet<WorkspaceId> = HashSet::new();
        let mut missing = Vec::new();
        {
            let cache = self.sessions.session_meta_cache.lock().await;
            for session_id in &active_sessions {
                if let Some(entry) = cache.get(session_id) {
                    active_workspaces.insert(entry.value.workspace_id);
                } else {
                    missing.push(*session_id);
                }
            }
        }
        for session_id in missing {
            if let Ok(Some(workspace_id)) = self
                .global_store()
                .get_workspace_id_for_session(session_id)
                .await
            {
                active_workspaces.insert(workspace_id);
            }
        }

        stats.workspace_stores_evicted = self
            .core
            .stores
            .evict_idle_workspaces(config.workspace_ttl, &active_workspaces)
            .await;

        self.emit_cache_evicted("session_head", stats.session_head_evicted)
            .await;
        self.emit_cache_evicted("session_meta", stats.session_meta_evicted)
            .await;
        self.emit_cache_evicted("scheduler", stats.schedulers_evicted)
            .await;
        self.emit_cache_evicted("broadcaster", stats.broadcasters_evicted)
            .await;
        self.emit_cache_evicted("session_event_head", stats.session_event_heads_evicted)
            .await;
        self.emit_cache_evicted("file_completions", stats.file_completions_evicted)
            .await;
        self.emit_cache_evicted(
            "workspace_file_completions",
            stats.workspace_file_completions_evicted,
        )
        .await;
        self.emit_cache_evicted("git_status", stats.git_status_evicted)
            .await;
        self.emit_cache_evicted(
            "workspace_active_snapshot",
            stats.workspace_snapshot_evicted,
        )
        .await;
        self.emit_cache_evicted("workspace_active_heads", stats.workspace_heads_evicted)
            .await;
        self.emit_cache_evicted("worktree_bootstrap", stats.worktree_bootstrap_evicted)
            .await;
        self.emit_cache_evicted("workspace_store", stats.workspace_stores_evicted)
            .await;

        stats
    }

    fn find_running_install_locked(
        &self,
        installs: &mut HashMap<InstallId, InstallState>,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        installs.iter_mut().find_map(|(id, st)| {
            let _ = self.reconcile_stale_running_install_locked(*id, st);
            if st.provider_id == provider_id
                && st.target == target
                && matches!(st.state, InstallStateKind::Running)
            {
                Some(*id)
            } else {
                None
            }
        })
    }

    fn push_install_event_locked(st: &mut InstallState, event: InstallProgressEvent) {
        st.progress_pct =
            crate::installs::heuristic_progress_pct_from_event(&event, st.progress_pct);
        if st.events.len() >= 256 {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        let _ = st.tx.send(event);
    }

    fn set_install_info_event_override_locked(st: &mut InstallState, event: &InstallProgressEvent) {
        st.info_event_override = Some(event.clone());
        st.info_event_override_until =
            Some(event.at + chrono::Duration::milliseconds(PREREQUISITE_PROGRESS_VISIBILITY_MS));
    }

    fn mirrored_install_event(
        source_install_id: InstallId,
        source_provider_id: &str,
        source_event: &InstallProgressEvent,
        mirror_install_id: InstallId,
        mirror_state: &InstallState,
    ) -> InstallProgressEvent {
        InstallProgressEvent {
            install_id: mirror_install_id,
            provider_id: mirror_state.provider_id.clone(),
            target: mirror_state.target,
            at: chrono::Utc::now(),
            stage: PREREQUISITE_PROGRESS_STAGE_FLOOR.to_string(),
            message: format!(
                "Prerequisite {source_provider_id} (install {source_install_id}, stage {}): {}",
                source_event.stage, source_event.message
            ),
            level: source_event.level,
            bytes: source_event.bytes,
            total_bytes: source_event.total_bytes,
            attempt: source_event.attempt,
            error_code: source_event.error_code,
        }
    }

    pub async fn set_install_progress_pct_override(&self, install_id: InstallId, pct: Option<u8>) {
        let mut installs = self.providers.installs.lock().await;
        let Some(state) = installs.get_mut(&install_id) else {
            return;
        };
        state.progress_pct_override = pct;
    }

    pub async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        let mut map = self.providers.installs.lock().await;
        self.find_running_install_locked(&mut map, provider_id, target)
    }

    pub async fn start_install(
        &self,
        provider_id: String,
        target: Option<InstallTarget>,
    ) -> (InstallId, bool) {
        let _start_gate = self.providers.install_start_gate.lock().await;
        let existing = {
            let mut installs = self.providers.installs.lock().await;
            self.find_running_install_locked(&mut installs, &provider_id, target)
        };
        if let Some(existing) = existing {
            let mut event = OpsEvent::new("info", "provider_install_joined");
            event.provider_id = Some(provider_id.clone());
            event.meta = Some(serde_json::json!({
                "install_id": existing.to_string(),
                "target": target.map(|value| value.as_str()),
            }));
            self.telemetry.ops_events.emit(event);
            return (existing, false);
        }
        let install_id = InstallId::new_v4();
        let state = InstallState::new(provider_id, target);
        let provider_id = state.provider_id.clone();
        let target = state.target;
        self.providers
            .installs
            .lock()
            .await
            .insert(install_id, state);
        let mut event = OpsEvent::new("info", "provider_install_started");
        event.provider_id = Some(provider_id);
        event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": target.map(|value| value.as_str()),
        }));
        self.telemetry.ops_events.emit(event);
        (install_id, true)
    }

    pub async fn get_install_sender(
        &self,
        install_id: InstallId,
    ) -> Option<broadcast::Sender<InstallProgressEvent>> {
        self.providers
            .installs
            .lock()
            .await
            .get(&install_id)
            .map(|s| s.tx.clone())
    }

    pub async fn get_install_info(
        &self,
        install_id: InstallId,
    ) -> Option<crate::installs::InstallInfo> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        let _ = self.reconcile_stale_running_install_locked(install_id, st);
        Some(st.info(install_id))
    }

    pub async fn get_install_polling_info(
        &self,
        install_id: InstallId,
    ) -> Option<crate::installs::InstallInfo> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        let _ = self.reconcile_stale_running_install_locked(install_id, st);
        Some(st.polling_info(install_id))
    }

    pub async fn get_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        let _ = self.reconcile_stale_running_install_locked(install_id, st);
        Some(st.events.iter().cloned().collect())
    }

    pub async fn register_install_progress_mirror(
        &self,
        source_install_id: InstallId,
        mirror_install_id: InstallId,
    ) -> bool {
        let mut installs = self.providers.installs.lock().await;
        let (source_provider_id, source_target, inserted, last_event) = {
            let Some(source_state) = installs.get_mut(&source_install_id) else {
                return false;
            };
            let inserted = source_state.mirrors.insert(mirror_install_id);
            let source_provider_id = source_state.provider_id.clone();
            let source_target = source_state.target;
            let last_event = source_state.events.back().cloned();
            (source_provider_id, source_target, inserted, last_event)
        };
        let Some(mirror_state) = installs.get_mut(&mirror_install_id) else {
            return false;
        };
        if inserted {
            let source_event = last_event.unwrap_or_else(|| InstallProgressEvent {
                install_id: source_install_id,
                provider_id: source_provider_id.clone(),
                target: source_target,
                at: chrono::Utc::now(),
                stage: "start".to_string(),
                message: "Waiting for tracked prerequisite install to report progress".to_string(),
                level: InstallEventLevel::Info,
                bytes: None,
                total_bytes: None,
                attempt: None,
                error_code: None,
            });
            let mirrored_event = Self::mirrored_install_event(
                source_install_id,
                &source_provider_id,
                &source_event,
                mirror_install_id,
                mirror_state,
            );
            Self::set_install_info_event_override_locked(mirror_state, &mirrored_event);
            Self::push_install_event_locked(mirror_state, mirrored_event);
        }
        true
    }

    pub async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        let mut installs = self.providers.installs.lock().await;
        let Some(st) = installs.get_mut(&install_id) else {
            return;
        };
        let source_provider_id = st.provider_id.clone();
        let mirrors = st.mirrors.iter().copied().collect::<Vec<_>>();
        Self::push_install_event_locked(st, event.clone());
        for mirror_install_id in mirrors {
            let Some(mirror_state) = installs.get_mut(&mirror_install_id) else {
                continue;
            };
            let mirrored_event = Self::mirrored_install_event(
                install_id,
                &source_provider_id,
                &event,
                mirror_install_id,
                mirror_state,
            );
            Self::set_install_info_event_override_locked(mirror_state, &mirrored_event);
            Self::push_install_event_locked(mirror_state, mirrored_event);
        }
    }

    pub async fn finish_install(
        &self,
        install_id: InstallId,
        ok: bool,
        error: Option<String>,
        error_code: Option<InstallErrorCode>,
    ) {
        let mut map = self.providers.installs.lock().await;
        let Some(st) = map.get_mut(&install_id) else {
            return;
        };
        if !matches!(st.state, InstallStateKind::Cancelled) {
            st.state = if ok {
                InstallStateKind::Succeeded
            } else {
                InstallStateKind::Failed
            };
        }
        if !ok || matches!(st.state, InstallStateKind::Cancelled) {
            st.error = error.or_else(|| {
                if matches!(st.state, InstallStateKind::Cancelled) {
                    Some("Install canceled by user".to_string())
                } else {
                    None
                }
            });
            st.error_code = error_code.or({
                if matches!(st.state, InstallStateKind::Cancelled) {
                    Some(InstallErrorCode::Cancelled)
                } else {
                    None
                }
            });
        } else {
            st.error = None;
            st.error_code = None;
        }
        if ok {
            st.progress_pct = Some(100);
        }
        st.progress_pct_override = None;
        st.info_event_override = None;
        st.info_event_override_until = None;
        st.finished_at = Some(chrono::Utc::now());
        let provider_id = st.provider_id.clone();
        let target = st.target;
        let state = st.state;
        let error = st.error.clone();
        let error_code = st.error_code;
        drop(map);

        let event_name = match state {
            InstallStateKind::Succeeded => "provider_install_succeeded",
            InstallStateKind::Failed => "provider_install_failed",
            InstallStateKind::Cancelled => "provider_install_cancelled",
            InstallStateKind::Running => "provider_install_running",
        };
        let mut event = OpsEvent::new(
            if matches!(state, InstallStateKind::Failed) {
                "warn"
            } else {
                "info"
            },
            event_name,
        );
        event.provider_id = Some(provider_id);
        event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": target.map(|value| value.as_str()),
            "state": match state {
                InstallStateKind::Running => "running",
                InstallStateKind::Succeeded => "succeeded",
                InstallStateKind::Failed => "failed",
                InstallStateKind::Cancelled => "cancelled",
            },
            "error": error,
            "error_code": error_code.and_then(|value| serde_json::to_value(value).ok()),
            "ok": ok,
        }));
        self.telemetry.ops_events.emit(event);
    }

    pub async fn is_install_cancelled(&self, install_id: InstallId) -> bool {
        let map = self.providers.installs.lock().await;
        map.get(&install_id)
            .map(|st| matches!(st.state, InstallStateKind::Cancelled))
            .unwrap_or(false)
    }

    pub async fn cancel_install(
        &self,
        install_id: InstallId,
    ) -> Option<crate::installs::InstallInfo> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        if !matches!(st.state, InstallStateKind::Running) {
            return Some(st.info(install_id));
        }

        st.state = InstallStateKind::Cancelled;
        st.error = Some("Install canceled by user".to_string());
        st.error_code = Some(InstallErrorCode::Cancelled);
        st.progress_pct_override = None;
        st.info_event_override = None;
        st.info_event_override_until = None;
        st.finished_at = Some(chrono::Utc::now());

        let event = InstallProgressEvent {
            install_id,
            provider_id: st.provider_id.clone(),
            target: st.target,
            at: chrono::Utc::now(),
            stage: "cancelled".to_string(),
            message: "Install canceled by user".to_string(),
            level: InstallEventLevel::Warning,
            bytes: None,
            total_bytes: None,
            attempt: None,
            error_code: Some(InstallErrorCode::Cancelled),
        };
        if st.events.len() >= 256 {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        let _ = st.tx.send(event);
        let provider_id = st.provider_id.clone();
        let target = st.target;
        drop(map);

        let mut ops_event = OpsEvent::new("info", "provider_install_cancel_requested");
        ops_event.provider_id = Some(provider_id);
        ops_event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": target.map(|value| value.as_str()),
        }));
        self.telemetry.ops_events.emit(ops_event);

        self.get_install_info(install_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_state(temp: &tempfile::TempDir) -> Arc<AppState> {
        Arc::new(AppState::new(
            temp.path().to_path_buf(),
            StoreManager::open(temp.path()).await.expect("open stores"),
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        ))
    }

    #[tokio::test]
    async fn find_running_install_reconciles_stale_running_venv_install() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = test_state(&temp).await;
        let install_id = InstallId::new_v4();
        let now = chrono::Utc::now();
        let mut install = InstallState::new("mistral".to_string(), Some(InstallTarget::Container));
        install.started_at = now - chrono::Duration::minutes(9);
        install.events.push_back(InstallProgressEvent {
            install_id,
            provider_id: "mistral".to_string(),
            target: Some(InstallTarget::Container),
            at: now - chrono::Duration::minutes(8),
            stage: "venv".to_string(),
            message: "Creating virtualenv…".to_string(),
            level: InstallEventLevel::Info,
            bytes: None,
            total_bytes: None,
            attempt: None,
            error_code: None,
        });
        state
            .providers
            .installs
            .lock()
            .await
            .insert(install_id, install);

        let running = state
            .find_running_install("mistral", Some(InstallTarget::Container))
            .await;
        assert!(running.is_none());

        let info = state
            .get_install_info(install_id)
            .await
            .expect("missing install info");
        assert!(matches!(info.state, InstallStateKind::Failed));
        assert_eq!(info.error_code, Some(InstallErrorCode::Timeout));
        assert!(info
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("timed out during venv"));
    }

    #[tokio::test]
    async fn get_install_info_preserves_recent_running_install() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = test_state(&temp).await;
        let install_id = InstallId::new_v4();
        let now = chrono::Utc::now();
        let mut install = InstallState::new("codex".to_string(), Some(InstallTarget::Container));
        install.started_at = now - chrono::Duration::minutes(1);
        install.events.push_back(InstallProgressEvent {
            install_id,
            provider_id: "codex".to_string(),
            target: Some(InstallTarget::Container),
            at: now - chrono::Duration::seconds(30),
            stage: "download".to_string(),
            message: "downloading…".to_string(),
            level: InstallEventLevel::Info,
            bytes: Some(10),
            total_bytes: Some(100),
            attempt: None,
            error_code: None,
        });
        state
            .providers
            .installs
            .lock()
            .await
            .insert(install_id, install);

        let info = state
            .get_install_info(install_id)
            .await
            .expect("missing install info");
        assert!(matches!(info.state, InstallStateKind::Running));
        assert_eq!(info.error_code, None);
    }

    #[tokio::test]
    async fn start_install_dedupes_concurrent_requests_for_same_provider_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = test_state(&temp).await;

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let state = state.clone();
            tasks.push(tokio::spawn(async move {
                state
                    .start_install("acp-crp-bridge".to_string(), Some(InstallTarget::Container))
                    .await
            }));
        }

        let mut install_ids = Vec::new();
        let mut started_new_count = 0usize;
        for task in tasks {
            let (install_id, started_new) = task.await.expect("join start_install task");
            install_ids.push(install_id);
            if started_new {
                started_new_count += 1;
            }
        }

        assert_eq!(
            started_new_count, 1,
            "concurrent start_install callers must share one tracked running install"
        );
        assert!(
            install_ids
                .windows(2)
                .all(|pair| pair.first() == pair.get(1)),
            "all concurrent start_install callers should receive the same install id: {install_ids:#?}"
        );
    }
}
