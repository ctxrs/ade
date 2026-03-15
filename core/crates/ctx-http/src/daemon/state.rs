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

mod installs;
mod metrics;
mod types;

use super::edit_plans;
pub(crate) use types::{
    ActiveHeadProjectionEntry, ActiveTaskRefreshEntry, AttachmentMaterializationTask,
    WorktreeBootstrapGate,
};
pub use types::{
    AppState, CacheSweepConfig, CacheSweepStats, CachedFileCompletions, CachedProviderOptions,
    CachedProviderVerify, CoreState, ExecutionRuntime, GitStatusSnapshotCacheEntry,
    ProviderRuntime, SessionHeadCacheKey, SessionRuntime, TelemetryRuntime, TimedEntry,
    TransportRuntime, WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
    WorkspaceRuntime, WorktreeVcsSnapshotCacheEntry,
};

impl AppState {
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
        let storage_guard = crate::storage_guard::StorageGuardRuntime::new(&data_root);
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
                storage_guard,
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
                kimi_login_sessions: Mutex::new(HashMap::new()),
                cursor_login_sessions: Mutex::new(HashMap::new()),
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
}

#[cfg(test)]
mod tests;
