use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::Router;
use chrono::Utc;
use directories::BaseDirs;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, watch, Mutex, Notify};

mod auth;
mod edit_plans;

use crate::buffers::BufferStore;
use crate::edit_plans::{EditPlan, EditPlanId};
use ctx_core::ids::{MessageId, SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Message, MessageAttachment, MessageDelivery, MessageRole, Session, SessionEvent,
    SessionEventType, SessionHeadDelta, SessionHeadSnapshot, SessionSnapshot, SessionTurn,
    SessionTurnStatus, Task, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot,
    WorkspaceTaskSummary, Worktree,
};
use ctx_lsp::Language as LspLanguage;
use ctx_lsp::{LspManager, LspManagerConfig};
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::adapters::ProviderStatus;
use ctx_providers::ask_user_question::AskUserQuestionBroker;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_providers::tier1::Tier1AcpAdapter;
use ctx_store::{Store, StoreManager, StoreManagerConfig};

use crate::api;
use crate::installer;
use crate::installs::{InstallId, InstallProgressEvent, InstallState, InstallStateKind};
use crate::mobile_tunnel::MobileTunnelManager;
use crate::ops_events::OpsEvents;
use crate::perf_telemetry::PerfTelemetry;
use crate::provider_accounts;
use crate::provider_child_reclassifier;
use crate::provider_debug::apply_acp_heap_profile_env;
use crate::provider_guard;
use crate::provider_restart;
use crate::provider_usage;
use crate::resource_governance::{self, ResourceGovernanceRuntime};
use crate::resource_telemetry;
use crate::resource_utilization::ResourceSampler;
use crate::scheduler::{reconcile_turn_terminal_state, session_worker, SchedulerCommand};
use crate::settings;
use crate::telemetry::{Telemetry, TelemetryConfig};
use crate::terminals::TerminalManager;
use crate::tool_cgroup;
use crate::web_sessions::WebSessionManager;
use crate::workspace_active_snapshot::WorkspaceActiveSnapshotHub;

const ARCHIVED_SNAPSHOT_HEAD_LIMIT: u32 = 50;
const ACTIVE_HEAD_PROJECTION_DEBOUNCE_MS: u64 = 200;
const ACTIVE_HEAD_PROJECTION_MAX_FLUSH_MS: u64 = 1500;
const ACTIVE_TASK_REFRESH_DEBOUNCE_MS: u64 = 250;

fn active_head_projection_wait_duration(
    now: Instant,
    last_event_at: Instant,
    last_flush_at: Instant,
    debounce: Duration,
    max_flush: Duration,
) -> Duration {
    let wait_for_debounce = debounce.saturating_sub(now.duration_since(last_event_at));
    let wait_for_max = max_flush.saturating_sub(now.duration_since(last_flush_at));
    wait_for_debounce.min(wait_for_max)
}

fn active_head_projection_should_flush(
    now: Instant,
    last_event_at: Instant,
    last_flush_at: Instant,
    debounce: Duration,
    max_flush: Duration,
) -> bool {
    now.duration_since(last_event_at) >= debounce || now.duration_since(last_flush_at) >= max_flush
}

fn message_from_event(event: &SessionEvent, session: &Session) -> Option<Message> {
    let message_id = event
        .payload_json
        .get("message_id")
        .and_then(|v| v.as_str())
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .map(MessageId)?;
    let content = event
        .payload_json
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;
    let delivery = event
        .payload_json
        .get("delivery")
        .and_then(|v| serde_json::from_value::<MessageDelivery>(v.clone()).ok())
        .unwrap_or(MessageDelivery::Immediate);
    let attachments = event
        .payload_json
        .get("attachments")
        .and_then(|v| serde_json::from_value::<Vec<MessageAttachment>>(v.clone()).ok())
        .unwrap_or_default();
    let role = match event.event_type {
        SessionEventType::UserMessage => MessageRole::User,
        SessionEventType::AssistantMessageInserted => MessageRole::Assistant,
        _ => return None,
    };
    let delivered_at = match role {
        MessageRole::Assistant => Some(event.created_at),
        _ => None,
    };
    Some(Message {
        id: message_id,
        session_id: event.session_id,
        task_id: session.task_id,
        run_id: event.run_id,
        turn_id: event.turn_id,
        turn_sequence: event
            .payload_json
            .get("turn_sequence")
            .and_then(|v| v.as_i64()),
        role,
        content,
        attachments,
        delivery,
        delivered_at,
        created_at: event.created_at,
    })
}

fn turn_from_event(event: &SessionEvent, message: Option<&Message>) -> Option<SessionTurn> {
    if !matches!(event.event_type, SessionEventType::UserMessage) {
        return None;
    }
    let turn_id = event.turn_id?;
    let delivery = message
        .map(|msg| msg.delivery.clone())
        .unwrap_or(MessageDelivery::Immediate);
    let status = if matches!(delivery, MessageDelivery::Queued) {
        SessionTurnStatus::Queued
    } else {
        SessionTurnStatus::Running
    };
    Some(SessionTurn {
        turn_id,
        session_id: event.session_id,
        run_id: event.run_id,
        user_message_id: message.map(|msg| msg.id),
        status,
        start_seq: Some(event.seq),
        end_seq: None,
        started_at: event.created_at,
        updated_at: event.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::{active_head_projection_should_flush, active_head_projection_wait_duration};
    use std::time::{Duration, Instant};

    #[test]
    fn active_head_projection_waits_for_debounce_or_max_flush() {
        let debounce = Duration::from_millis(200);
        let max_flush = Duration::from_millis(1500);
        let now = Instant::now();

        let wait = active_head_projection_wait_duration(
            now,
            now - Duration::from_millis(100),
            now - Duration::from_millis(100),
            debounce,
            max_flush,
        );
        assert_eq!(wait, Duration::from_millis(100));

        let wait = active_head_projection_wait_duration(
            now,
            now - Duration::from_millis(50),
            now - Duration::from_millis(1490),
            debounce,
            max_flush,
        );
        assert_eq!(wait, Duration::from_millis(10));
    }

    #[test]
    fn active_head_projection_flushes_on_idle_or_max() {
        let debounce = Duration::from_millis(200);
        let max_flush = Duration::from_millis(1500);
        let now = Instant::now();

        assert!(active_head_projection_should_flush(
            now,
            now - Duration::from_millis(250),
            now - Duration::from_millis(500),
            debounce,
            max_flush
        ));
        assert!(active_head_projection_should_flush(
            now,
            now - Duration::from_millis(50),
            now - Duration::from_millis(1600),
            debounce,
            max_flush
        ));
        assert!(!active_head_projection_should_flush(
            now,
            now - Duration::from_millis(50),
            now - Duration::from_millis(500),
            debounce,
            max_flush
        ));
    }
}

pub struct AppState {
    pub data_root: PathBuf,
    pub tool_output_spool_enabled: bool,
    pub tool_output_spool_dir: PathBuf,
    pub stores: StoreManager,
    pub providers: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub provider_statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub provider_matrix_cache: Mutex<crate::provider_matrix::ProviderMatrixCache>,
    pub provider_options_cache: Mutex<HashMap<String, CachedProviderOptions>>,
    pub provider_verify_cache: Mutex<HashMap<String, CachedProviderVerify>>,
    pub file_completions_cache: Mutex<HashMap<WorktreeId, CachedFileCompletions>>,
    pub workspace_file_completions_cache: Mutex<HashMap<WorkspaceId, CachedFileCompletions>>,
    pub git_status_snapshots: Mutex<HashMap<WorktreeId, GitStatusSnapshotCacheEntry>>,
    pub git_status_watchers: Mutex<HashSet<WorktreeId>>,
    pub daemon_url: String,
    pub auth_token: Option<String>,
    pub lsp_cfg: LspManagerConfig,
    pub lsp: Arc<LspManager>,
    pub lsp_edit_plans_enabled: bool,
    pub buffers: BufferStore,
    pub ask_user_question: Arc<AskUserQuestionBroker>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub telemetry: Telemetry,
    pub ops_events: OpsEvents,
    pub perf_telemetry: PerfTelemetry,
    pub resource_governance: Mutex<ResourceGovernanceRuntime>,
    pub provider_guard: Mutex<provider_guard::ProviderGuardRuntime>,
    pub provider_restart: Mutex<provider_restart::ProviderRestartRuntime>,
    pub provider_usage_cache: Mutex<HashMap<String, provider_usage::ProviderUsageSnapshot>>,
    pub codex_login_sessions: Mutex<HashMap<String, provider_accounts::CodexLoginStatus>>,
    pub resource_sampler: Mutex<ResourceSampler>,
    pub workspace_active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    pub workspace_active_snapshot_cache:
        Mutex<HashMap<WorkspaceId, WorkspaceActiveSnapshotCacheEntry>>,
    pub workspace_active_heads_cache: Mutex<HashMap<WorkspaceId, WorkspaceActiveHeadCacheEntry>>,
    pub session_head_cache:
        Mutex<HashMap<SessionId, HashMap<SessionHeadCacheKey, SessionHeadSnapshot>>>,
    pub terminals: TerminalManager,
    pub mobile_tunnel: MobileTunnelManager,
    pub web_sessions: Arc<WebSessionManager>,
    pub merge_queue_notify: Arc<Notify>,
    schedulers: Mutex<HashMap<SessionId, mpsc::Sender<SchedulerCommand>>>,
    broadcasters: Mutex<HashMap<SessionId, broadcast::Sender<SessionEvent>>>,
    session_event_heads: Mutex<HashMap<SessionId, watch::Sender<i64>>>,
    active_head_projections: Mutex<HashMap<SessionId, ActiveHeadProjectionEntry>>,
    active_task_refreshes: Mutex<HashMap<TaskId, ActiveTaskRefreshEntry>>,
    lsp_diag_broadcaster: broadcast::Sender<serde_json::Value>,
    lsp_diag_forwarders: Mutex<HashSet<String>>,
    running_sessions: Mutex<HashSet<SessionId>>,
    installs: Mutex<HashMap<InstallId, InstallState>>,
    pub edit_plans: Mutex<HashMap<EditPlanId, EditPlan>>,
    session_meta_cache: Mutex<HashMap<SessionId, Session>>,
    worktree_bootstrap_gates: Mutex<HashMap<WorktreeId, WorktreeBootstrapGate>>,
}

struct WorktreeBootstrapGate {
    wait_for_completion: bool,
    done_tx: watch::Sender<bool>,
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
struct ActiveHeadProjectionEntry {
    last_event_seq: i64,
    last_event_at: Instant,
    last_flushed_seq: i64,
    last_flush_at: Instant,
}

struct ActiveTaskRefreshEntry {
    generation: u64,
}

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
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
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
        let tool_output_spool_enabled = std::env::var("CTX_TOOL_OUTPUT_DISK_SPOOL")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
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
        let workspace_active_snapshot = Arc::new(WorkspaceActiveSnapshotHub::new());
        let web_sessions = Arc::new(WebSessionManager::new());
        let merge_queue_notify = Arc::new(Notify::new());
        Self {
            data_root,
            tool_output_spool_enabled,
            tool_output_spool_dir,
            stores,
            providers: Mutex::new(providers),
            provider_statuses: Mutex::new(HashMap::new()),
            provider_matrix_cache: Mutex::new(
                crate::provider_matrix::ProviderMatrixCache::default(),
            ),
            provider_options_cache: Mutex::new(HashMap::new()),
            provider_verify_cache: Mutex::new(HashMap::new()),
            file_completions_cache: Mutex::new(HashMap::new()),
            workspace_file_completions_cache: Mutex::new(HashMap::new()),
            git_status_snapshots: Mutex::new(HashMap::new()),
            git_status_watchers: Mutex::new(HashSet::new()),
            daemon_url,
            auth_token,
            lsp_cfg,
            lsp,
            lsp_edit_plans_enabled,
            buffers: BufferStore::default(),
            ask_user_question,
            shutdown_tx,
            telemetry,
            ops_events,
            perf_telemetry,
            resource_governance: Mutex::new(ResourceGovernanceRuntime::default()),
            provider_guard: Mutex::new(provider_guard::ProviderGuardRuntime::default()),
            provider_restart: Mutex::new(provider_restart::ProviderRestartRuntime::default()),
            provider_usage_cache: Mutex::new(HashMap::new()),
            codex_login_sessions: Mutex::new(HashMap::new()),
            resource_sampler: Mutex::new(ResourceSampler::new()),
            workspace_active_snapshot,
            workspace_active_snapshot_cache: Mutex::new(HashMap::new()),
            workspace_active_heads_cache: Mutex::new(HashMap::new()),
            session_head_cache: Mutex::new(HashMap::new()),
            terminals: TerminalManager::default(),
            mobile_tunnel: MobileTunnelManager::default(),
            web_sessions,
            merge_queue_notify,
            schedulers: Mutex::new(HashMap::new()),
            broadcasters: Mutex::new(HashMap::new()),
            session_event_heads: Mutex::new(HashMap::new()),
            active_head_projections: Mutex::new(HashMap::new()),
            active_task_refreshes: Mutex::new(HashMap::new()),
            lsp_diag_broadcaster,
            lsp_diag_forwarders: Mutex::new(HashSet::new()),
            running_sessions: Mutex::new(HashSet::new()),
            installs: Mutex::new(HashMap::new()),
            edit_plans: Mutex::new(edit_plans),
            session_meta_cache: Mutex::new(HashMap::new()),
            worktree_bootstrap_gates: Mutex::new(HashMap::new()),
        }
    }

    pub fn edit_plans_dir(&self) -> PathBuf {
        edit_plans::edit_plans_dir(&self.data_root)
    }

    pub fn persist_edit_plan(&self, plan: &EditPlan) {
        if let Err(e) = edit_plans::persist_edit_plan_to_disk(&self.data_root, plan) {
            tracing::warn!("failed to persist edit plan {}: {e}", plan.id.0);
        }
    }

    pub fn delete_edit_plan_file(&self, plan_id: EditPlanId) {
        if let Err(e) = edit_plans::delete_edit_plan_file(&self.data_root, plan_id) {
            tracing::warn!("failed to delete edit plan {} file: {e}", plan_id.0);
        }
    }

    pub fn global_store(&self) -> &Store {
        self.stores.global()
    }

    pub async fn store_for_workspace(&self, workspace_id: WorkspaceId) -> Result<Store> {
        self.stores.workspace(workspace_id).await
    }

    pub async fn store_for_task(&self, task_id: TaskId) -> Result<Store> {
        self.stores.store_for_task(task_id).await
    }

    pub async fn store_for_session(&self, session_id: SessionId) -> Result<Store> {
        self.stores.store_for_session(session_id).await
    }

    pub async fn store_for_worktree(&self, worktree_id: WorktreeId) -> Result<Store> {
        self.stores.store_for_worktree(worktree_id).await
    }

    pub async fn cached_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<(i64, i64)> {
        let cache = self.workspace_active_snapshot_cache.lock().await;
        cache
            .get(&workspace_id)
            .map(|entry| (entry.snapshot.snapshot_rev, entry.snapshot.archived_rev))
    }

    pub async fn cached_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveSnapshot> {
        let cache = self.workspace_active_snapshot_cache.lock().await;
        cache.get(&workspace_id).map(|entry| entry.snapshot.clone())
    }

    pub async fn cache_workspace_active_snapshot(&self, snapshot: WorkspaceActiveSnapshot) {
        let workspace_id = snapshot.workspace_id;
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.insert(workspace_id, WorkspaceActiveSnapshotCacheEntry { snapshot });
    }

    pub async fn cached_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveHeadBatch> {
        let cache = self.workspace_active_heads_cache.lock().await;
        cache.get(&workspace_id).map(|entry| entry.batch.clone())
    }

    pub async fn cache_workspace_active_heads(&self, batch: WorkspaceActiveHeadBatch) {
        let workspace_id = batch.workspace_id;
        let mut cache = self.workspace_active_heads_cache.lock().await;
        cache.insert(workspace_id, WorkspaceActiveHeadCacheEntry { batch });
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(&self, workspace_id: WorkspaceId) {
        if !self
            .workspace_active_snapshot
            .needs_hydration(workspace_id)
            .await
        {
            return;
        }
        let store = match self.store_for_workspace(workspace_id).await {
            Ok(store) => store,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot (store lookup)"
                );
                return;
            }
        };
        let (_, archived_rev) = match store
            .get_workspace_active_snapshot_state(workspace_id)
            .await
        {
            Ok(state) => state,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot state"
                );
                return;
            }
        };
        let (tasks, _) = match store
            .list_workspace_active_page_base(workspace_id, i64::MAX)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot tasks"
                );
                return;
            }
        };
        let session_ids = match store.list_workspace_active_session_ids(workspace_id).await {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot session ids"
                );
                return;
            }
        };
        let mut heads = Vec::new();
        for session_id in session_ids {
            if let Ok(Some(head)) = store.get_active_snapshot_head(session_id).await {
                heads.push(head);
            }
        }
        self.workspace_active_snapshot
            .hydrate_snapshot(workspace_id, 0, archived_rev, tasks, heads)
            .await;
    }

    pub async fn cached_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Option<SessionHeadSnapshot> {
        let cache = self.session_head_cache.lock().await;
        cache
            .get(&session_id)
            .and_then(|by_key| {
                by_key.get(&SessionHeadCacheKey {
                    limit,
                    include_events,
                })
            })
            .cloned()
    }

    pub async fn cache_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
        snapshot: SessionHeadSnapshot,
    ) {
        let mut cache = self.session_head_cache.lock().await;
        cache.entry(session_id).or_insert_with(HashMap::new).insert(
            SessionHeadCacheKey {
                limit,
                include_events,
            },
            snapshot,
        );
    }

    pub async fn get_broadcaster(&self, session_id: SessionId) -> broadcast::Sender<SessionEvent> {
        let mut map = self.broadcasters.lock().await;
        map.entry(session_id)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(256);
                tx
            })
            .clone()
    }

    pub async fn subscribe_session_event_head(
        &self,
        session_id: SessionId,
    ) -> watch::Receiver<i64> {
        let mut map = self.session_event_heads.lock().await;
        if let Some(tx) = map.get(&session_id) {
            return tx.subscribe();
        }
        let (tx, rx) = watch::channel::<i64>(0);
        map.insert(session_id, tx);
        rx
    }

    pub fn lsp_diag_broadcaster(&self) -> broadcast::Sender<serde_json::Value> {
        self.lsp_diag_broadcaster.clone()
    }

    pub async fn publish_event(self: &Arc<Self>, event: SessionEvent) {
        let tx = self.get_broadcaster(event.session_id).await;
        let _ = tx.send(event.clone());
        let mut map = self.session_event_heads.lock().await;
        let sender = map.entry(event.session_id).or_insert_with(|| {
            let (tx, _rx) = watch::channel::<i64>(0);
            tx
        });
        let _ = sender.send(event.seq);
        if !matches!(
            event.event_type,
            SessionEventType::AssistantChunk | SessionEventType::ThoughtChunk
        ) {
            self.queue_active_head_projection(event.session_id, event.seq)
                .await;
        }
        self.update_workspace_active_snapshot_for_event(&event)
            .await;
    }

    async fn update_workspace_active_snapshot_for_event(self: &Arc<Self>, event: &SessionEvent) {
        let session = {
            let cache = self.session_meta_cache.lock().await;
            cache.get(&event.session_id).cloned()
        };
        let session = match session {
            Some(session) => session,
            None => {
                let store = match self.store_for_session(event.session_id).await {
                    Ok(store) => store,
                    Err(_) => return,
                };
                let Some(session) = store.get_session(event.session_id).await.ok().flatten() else {
                    return;
                };
                self.remember_session_meta(&session).await;
                session
            }
        };

        let stream_only = matches!(
            event.event_type,
            SessionEventType::AssistantChunk | SessionEventType::ThoughtChunk
        );

        let update_task = matches!(
            event.event_type,
            SessionEventType::UserMessage
                | SessionEventType::AssistantMessageInserted
                | SessionEventType::AssistantComplete
                | SessionEventType::Done
                | SessionEventType::TurnQueued
                | SessionEventType::TurnStarted
                | SessionEventType::TurnFinished
                | SessionEventType::TurnInterrupted
                | SessionEventType::MessageQueueAdded
                | SessionEventType::MessageQueueUpdated
                | SessionEventType::MessageQueueRemoved
                | SessionEventType::MessageQueuePromoted
                | SessionEventType::Error
        );
        if update_task {
            self.queue_workspace_task_refresh(session.task_id).await;
        }

        let message = if matches!(
            event.event_type,
            SessionEventType::UserMessage
                | SessionEventType::AssistantMessageInserted
                | SessionEventType::Notice
        ) {
            message_from_event(event, &session)
        } else {
            None
        };
        let mut turn = turn_from_event(event, message.as_ref());
        if turn.is_none()
            && matches!(
                event.event_type,
                SessionEventType::TurnStarted | SessionEventType::TurnFinished
            )
        {
            if let Some(turn_id) = event.turn_id {
                if let Ok(store) = self.store_for_session(event.session_id).await {
                    if let Ok(Some(fetched)) =
                        store.get_session_turn(event.session_id, turn_id).await
                    {
                        turn = Some(fetched);
                    }
                }
            }
        }

        let last_event_seq = if stream_only {
            self.workspace_active_snapshot
                .session_last_event_seq(session.workspace_id, event.session_id)
                .await
        } else {
            event.seq
        };
        let state_rev = if stream_only {
            last_event_seq
        } else {
            event.seq
        };

        let delta = SessionHeadDelta {
            session_id: event.session_id,
            last_event_seq,
            state_rev,
            event: Some(event.clone()),
            turn,
            message,
        };
        self.workspace_active_snapshot
            .publish_session_head_delta(session.workspace_id, &session, delta, !stream_only)
            .await;
    }

    async fn queue_active_head_projection(
        self: &Arc<Self>,
        session_id: SessionId,
        last_event_seq: i64,
    ) {
        let now = Instant::now();
        let should_spawn = {
            let mut map = self.active_head_projections.lock().await;
            if let Some(entry) = map.get_mut(&session_id) {
                entry.last_event_seq = entry.last_event_seq.max(last_event_seq);
                entry.last_event_at = now;
                false
            } else {
                map.insert(
                    session_id,
                    ActiveHeadProjectionEntry {
                        last_event_seq,
                        last_event_at: now,
                        last_flushed_seq: 0,
                        last_flush_at: now,
                    },
                );
                true
            }
        };
        if should_spawn {
            let state = Arc::downgrade(self);
            tokio::spawn(async move {
                let Some(state) = state.upgrade() else {
                    return;
                };
                state.run_active_head_projection(session_id).await;
            });
        }
    }

    async fn queue_workspace_task_refresh(self: &Arc<Self>, task_id: TaskId) {
        let should_spawn = {
            let mut map = self.active_task_refreshes.lock().await;
            if let Some(entry) = map.get_mut(&task_id) {
                entry.generation = entry.generation.wrapping_add(1);
                false
            } else {
                map.insert(task_id, ActiveTaskRefreshEntry { generation: 1 });
                true
            }
        };
        if should_spawn {
            let state = Arc::downgrade(self);
            tokio::spawn(async move {
                let Some(state) = state.upgrade() else {
                    return;
                };
                state.run_workspace_task_refresh(task_id).await;
            });
        }
    }

    async fn run_active_head_projection(self: Arc<Self>, session_id: SessionId) {
        let debounce = Duration::from_millis(ACTIVE_HEAD_PROJECTION_DEBOUNCE_MS.max(1));
        let max_flush = Duration::from_millis(ACTIVE_HEAD_PROJECTION_MAX_FLUSH_MS.max(1));
        loop {
            let entry = {
                let map = self.active_head_projections.lock().await;
                map.get(&session_id).cloned()
            };
            let Some(entry) = entry else {
                return;
            };
            if entry.last_event_seq == entry.last_flushed_seq {
                tokio::time::sleep(debounce).await;
                let mut map = self.active_head_projections.lock().await;
                if let Some(entry) = map.get(&session_id) {
                    if entry.last_event_seq == entry.last_flushed_seq {
                        map.remove(&session_id);
                        return;
                    }
                } else {
                    return;
                }
                continue;
            }

            let wait_for = active_head_projection_wait_duration(
                Instant::now(),
                entry.last_event_at,
                entry.last_flush_at,
                debounce,
                max_flush,
            );
            if !wait_for.is_zero() {
                tokio::time::sleep(wait_for).await;
            }

            let entry = {
                let map = self.active_head_projections.lock().await;
                map.get(&session_id).cloned()
            };
            let Some(entry) = entry else {
                return;
            };
            if entry.last_event_seq == entry.last_flushed_seq {
                continue;
            }
            let now = Instant::now();
            if !active_head_projection_should_flush(
                now,
                entry.last_event_at,
                entry.last_flush_at,
                debounce,
                max_flush,
            ) {
                continue;
            }
            let target_seq = entry.last_event_seq;

            self.refresh_session_head_cache(session_id).await;

            let flushed_at = Instant::now();
            let mut map = self.active_head_projections.lock().await;
            match map.get_mut(&session_id) {
                Some(entry) => {
                    entry.last_flushed_seq = entry.last_flushed_seq.max(target_seq);
                    entry.last_flush_at = flushed_at;
                    if entry.last_event_seq == entry.last_flushed_seq {
                        map.remove(&session_id);
                        return;
                    }
                }
                None => return,
            }
        }
    }

    async fn run_workspace_task_refresh(self: Arc<Self>, task_id: TaskId) {
        let debounce = Duration::from_millis(ACTIVE_TASK_REFRESH_DEBOUNCE_MS.max(1));
        loop {
            let generation = {
                let map = self.active_task_refreshes.lock().await;
                match map.get(&task_id) {
                    Some(entry) => entry.generation,
                    None => return,
                }
            };

            tokio::time::sleep(debounce).await;

            let current = {
                let map = self.active_task_refreshes.lock().await;
                match map.get(&task_id) {
                    Some(entry) => entry.generation,
                    None => return,
                }
            };
            if current != generation {
                continue;
            }

            if let Err(err) = self.emit_workspace_task_upsert(task_id).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace active snapshot refresh failed: {err:?}"
                );
            }

            let mut map = self.active_task_refreshes.lock().await;
            match map.get(&task_id) {
                Some(entry) if entry.generation == current => {
                    map.remove(&task_id);
                    return;
                }
                Some(_) => continue,
                None => return,
            }
        }
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        let mut cache = self.session_meta_cache.lock().await;
        cache.insert(session.id, session.clone());
    }

    pub async fn refresh_session_head_cache(&self, session_id: SessionId) {
        let store = match self.store_for_session(session_id).await {
            Ok(store) => store,
            Err(_) => return,
        };
        let head = match store
            .get_session_head_snapshot(session_id, u32::MAX, true)
            .await
        {
            Ok(Some(head)) => head,
            Ok(None) => {
                self.workspace_active_snapshot
                    .remove_session_head(session_id)
                    .await;
                return;
            }
            Err(err) => {
                tracing::warn!(
                    session_id = %session_id.0,
                    "session head cache refresh failed: {err:#}"
                );
                return;
            }
        };
        self.workspace_active_snapshot
            .update_session_head(head)
            .await;
    }

    pub async fn register_worktree_bootstrap(
        &self,
        worktree_id: WorktreeId,
        wait_for_completion: bool,
    ) {
        let (done_tx, _) = watch::channel(false);
        let mut map = self.worktree_bootstrap_gates.lock().await;
        map.insert(
            worktree_id,
            WorktreeBootstrapGate {
                wait_for_completion,
                done_tx,
            },
        );
    }

    pub async fn finish_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        let gate = {
            let mut map = self.worktree_bootstrap_gates.lock().await;
            map.remove(&worktree_id)
        };
        if let Some(gate) = gate {
            let _ = gate.done_tx.send(true);
        }
    }

    pub async fn wait_for_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        let mut done_rx = {
            let map = self.worktree_bootstrap_gates.lock().await;
            let Some(gate) = map.get(&worktree_id) else {
                return;
            };
            if !gate.wait_for_completion {
                return;
            }
            gate.done_tx.subscribe()
        };
        if *done_rx.borrow() {
            return;
        }
        let _ = done_rx.changed().await;
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> Result<()> {
        let mut task: Option<Task> = None;
        let store = self.store_for_task(task_id).await?;
        match store.get_workspace_active_task_summary(task_id).await? {
            Some(summary) => {
                let workspace_id = summary.task.workspace_id;
                task = Some(summary.task.clone());
                self.workspace_active_snapshot
                    .publish_active_task_upsert(workspace_id, summary)
                    .await;
            }
            None => {
                if let Some(loaded) = store.get_task(task_id).await? {
                    task = Some(loaded.clone());
                    self.workspace_active_snapshot
                        .publish_active_task_delete(loaded.workspace_id, task_id)
                        .await;
                }
            }
        }

        if let Some(task) = task.as_ref().filter(|task| task.archived_at.is_some()) {
            let _ = self.emit_workspace_archived_task_upsert(task).await;
        }
        Ok(())
    }

    pub async fn emit_workspace_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        if let Err(err) = self.store_for_workspace(workspace_id).await {
            tracing::warn!(
                workspace_id = %workspace_id.0,
                task_id = %task_id.0,
                "workspace task delete store missing: {err:#}"
            );
            return;
        }
        self.workspace_active_snapshot
            .publish_active_task_delete(workspace_id, task_id)
            .await;
    }

    pub async fn emit_workspace_archived_task_delete(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        let updated = match self.store_for_workspace(workspace_id).await {
            Ok(store) => match store
                .bump_workspace_archived_snapshot_rev(workspace_id)
                .await
            {
                Ok(_) => true,
                Err(err) => {
                    tracing::warn!(
                        workspace_id = %workspace_id.0,
                        task_id = %task_id.0,
                        "workspace archived delete read model update failed: {err:#}"
                    );
                    false
                }
            },
            Err(err) => {
                tracing::warn!(
                    workspace_id = %workspace_id.0,
                    task_id = %task_id.0,
                    "workspace archived delete read model store missing: {err:#}"
                );
                false
            }
        };
        if updated {
            self.workspace_active_snapshot
                .publish_archived_task_delete(workspace_id, task_id)
                .await;
        }
    }

    async fn emit_workspace_archived_task_upsert(&self, task: &Task) -> Result<()> {
        let store = self.store_for_task(task.id).await?;
        let Some(summary) = store.get_workspace_task_summary(task.id).await? else {
            return Ok(());
        };
        if summary.task.archived_at.is_none() {
            return Ok(());
        }

        let primary_session_id = select_primary_session_id(&summary);
        let snapshot: Option<SessionSnapshot> = match primary_session_id {
            Some(session_id) => {
                let session_store = self.store_for_session(session_id).await?;
                session_store
                    .get_session_snapshot(session_id, ARCHIVED_SNAPSHOT_HEAD_LIMIT, false)
                    .await?
            }
            None => None,
        };
        let _ = store
            .bump_workspace_archived_snapshot_rev(task.workspace_id)
            .await?;
        self.workspace_active_snapshot
            .publish_archived_task_upsert(task.workspace_id, summary, snapshot)
            .await;
        Ok(())
    }

    pub async fn ensure_lsp_diagnostics_forwarder(
        self: &Arc<Self>,
        root: PathBuf,
        lang: LspLanguage,
    ) {
        if !self.lsp.enabled() {
            return;
        }
        let key = format!("{}:{}", root.to_string_lossy(), lang.id());
        {
            let mut set = self.lsp_diag_forwarders.lock().await;
            if set.contains(&key) {
                return;
            }
            set.insert(key);
        }

        let state = self.clone();
        tokio::spawn(async move {
            let mut rx = match state
                .lsp
                .subscribe_diagnostics_for_language(&root, lang)
                .await
            {
                Ok(v) => v,
                Err(_) => return,
            };
            loop {
                let update = match rx.recv().await {
                    Ok(u) => u,
                    Err(_) => break,
                };
                let url = match url::Url::parse(&update.uri.to_string()) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                if url.scheme() != "file" {
                    continue;
                }
                let Ok(abs_path) = url.to_file_path() else {
                    continue;
                };

                let watchers = state.buffers.watchers_for_abs_path(&abs_path).await;
                if watchers.is_empty() {
                    continue;
                }

                let diagnostics_json = match serde_json::to_value(&update.diagnostics) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                for (sid, rel) in watchers {
                    let msg = serde_json::json!({
                        "type": "lsp_diagnostics",
                        "session_id": sid.0.to_string(),
                        "path": rel.to_string_lossy(),
                        "diagnostics": diagnostics_json,
                    });
                    let _ = state.lsp_diag_broadcaster.send(msg);
                }
            }
        });
    }

    pub async fn ensure_git_status_watcher(self: &Arc<Self>, worktree: Worktree) {
        let mut watchers = self.git_status_watchers.lock().await;
        if !watchers.insert(worktree.id) {
            return;
        }
        let worktree_id = worktree.id;
        let state = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(err) =
                crate::git_status::run_git_status_watcher(state.clone(), worktree).await
            {
                tracing::warn!(worktree_id = %worktree_id.0, "git status watcher failed: {err:#}");
            }
            state.release_git_status_watcher(worktree_id).await;
        });
    }

    pub async fn release_git_status_watcher(&self, worktree_id: WorktreeId) {
        let mut watchers = self.git_status_watchers.lock().await;
        watchers.remove(&worktree_id);
    }

    pub async fn ensure_scheduler(
        self: &Arc<Self>,
        session: Session,
    ) -> mpsc::Sender<SchedulerCommand> {
        self.remember_session_meta(&session).await;
        let mut map = self.schedulers.lock().await;
        if let Some(tx) = map.get(&session.id) {
            return tx.clone();
        }
        let (tx, rx) = mpsc::channel(64);
        map.insert(session.id, tx.clone());
        tokio::spawn(session_worker(self.clone(), session, rx));
        tx
    }

    pub async fn scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<SchedulerCommand>> {
        self.schedulers.lock().await.get(&session_id).cloned()
    }

    pub async fn is_running(&self, session_id: SessionId) -> bool {
        self.running_sessions.lock().await.contains(&session_id)
    }

    pub async fn list_running_sessions(&self) -> Vec<SessionId> {
        self.running_sessions.lock().await.iter().copied().collect()
    }

    pub async fn set_running(&self, session_id: SessionId, running: bool) {
        let mut set = self.running_sessions.lock().await;
        if running {
            set.insert(session_id);
        } else {
            set.remove(&session_id);
        }
    }

    pub async fn find_running_install(&self, provider_id: &str) -> Option<InstallId> {
        let map = self.installs.lock().await;
        map.iter().find_map(|(id, st)| {
            if st.provider_id == provider_id && matches!(st.state, InstallStateKind::Running) {
                Some(*id)
            } else {
                None
            }
        })
    }

    pub async fn start_install(&self, provider_id: String) -> (InstallId, bool) {
        if let Some(existing) = self.find_running_install(&provider_id).await {
            return (existing, false);
        }
        let install_id = InstallId::new_v4();
        let state = InstallState::new(provider_id);
        self.installs.lock().await.insert(install_id, state);
        (install_id, true)
    }

    pub async fn get_install_sender(
        &self,
        install_id: InstallId,
    ) -> Option<broadcast::Sender<InstallProgressEvent>> {
        self.installs
            .lock()
            .await
            .get(&install_id)
            .map(|s| s.tx.clone())
    }

    pub async fn get_install_info(
        &self,
        install_id: InstallId,
    ) -> Option<crate::installs::InstallInfo> {
        self.installs
            .lock()
            .await
            .get(&install_id)
            .map(|s| s.info(install_id))
    }

    pub async fn get_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        self.installs
            .lock()
            .await
            .get(&install_id)
            .map(|s| s.events.iter().cloned().collect())
    }

    pub async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        let mut map = self.installs.lock().await;
        let Some(st) = map.get_mut(&install_id) else {
            return;
        };
        if st.events.len() >= 256 {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        let _ = st.tx.send(event);
    }

    pub async fn finish_install(&self, install_id: InstallId, ok: bool, error: Option<String>) {
        let mut map = self.installs.lock().await;
        let Some(st) = map.get_mut(&install_id) else {
            return;
        };
        st.state = if ok {
            InstallStateKind::Succeeded
        } else {
            InstallStateKind::Failed
        };
        st.error = error;
        st.finished_at = Some(Utc::now());
    }
}

fn select_primary_session_id(summary: &WorkspaceTaskSummary) -> Option<SessionId> {
    if let Some(primary_id) = summary.task.primary_session_id {
        if summary
            .sessions
            .iter()
            .any(|session| session.id == primary_id)
        {
            return Some(primary_id);
        }
    }
    summary
        .sessions
        .iter()
        .find(|session| session.parent_session_id.is_none())
        .map(|session| session.id)
        .or_else(|| summary.sessions.first().map(|session| session.id))
}

async fn reconcile_running_turns(state: &Arc<AppState>) -> Result<()> {
    let workspaces = state.global_store().list_workspaces().await?;
    let mut running_turns = Vec::new();
    for workspace in workspaces {
        let store = state.store_for_workspace(workspace.id).await?;
        let mut turns = store
            .list_session_turns_by_statuses(&[SessionTurnStatus::Running])
            .await?;
        running_turns.append(&mut turns);
    }

    for turn in running_turns {
        if let Err(err) = reconcile_turn_terminal_state(
            state,
            turn.session_id,
            turn.run_id,
            turn.turn_id,
            "daemon_restart",
        )
        .await
        {
            tracing::warn!(
                session_id = %turn.session_id.0,
                turn_id = %turn.turn_id.0,
                err = %err,
                "failed to reconcile running turn after daemon restart"
            );
        }
    }

    Ok(())
}

pub async fn serve(bind: String, data_dir: Option<String>) -> Result<()> {
    let data_root = match data_dir {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("resolving home dir")?;
            base.home_dir().join(".ctx")
        }
    };
    tokio::fs::create_dir_all(&data_root).await?;
    tokio::fs::create_dir_all(data_root.join("logs")).await.ok();

    let _daemon_lock = auth::acquire_daemon_lock(&data_root)?;

    let settings_data = settings::load_settings(&data_root).await;
    let store_config = settings_data
        .storage
        .as_ref()
        .map(|storage| StoreManagerConfig {
            max_connections: storage.max_connections,
        })
        .unwrap_or_default();
    let stores = StoreManager::open_with_config(&data_root, store_config).await?;

    // Retention policy (configurable via env):
    // - Keep tool summaries and final thoughts for archived tasks for N days.
    // - Do not retain thought chunk events (handled at ingestion time).
    const DEFAULT_TOOL_SUMMARY_RETENTION_DAYS: u64 = 30;
    fn tool_summary_retention_days() -> u64 {
        std::env::var("CTX_TOOL_SUMMARY_RETENTION_DAYS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_TOOL_SUMMARY_RETENTION_DAYS)
    }
    {
        let stores = stores.clone();
        tokio::spawn(async move {
            let mut last_cleanup = None::<String>;
            loop {
                let today = Utc::now().format("%Y-%m-%d").to_string();
                if last_cleanup.as_deref() != Some(&today) {
                    let retention_days = tool_summary_retention_days();
                    match stores.global().list_workspaces().await {
                        Ok(workspaces) => {
                            for workspace in workspaces {
                                match stores.workspace(workspace.id).await {
                                    Ok(store) => {
                                        match store
                                            .prune_session_data_older_than_days(retention_days)
                                            .await
                                        {
                                            Ok(stats) => {
                                                tracing::info!(
                                                    workspace_id = %workspace.id.0,
                                                    tool_summaries_deleted = stats
                                                        .tool_summaries_deleted,
                                                    turn_thoughts_cleared = stats
                                                        .turn_thoughts_cleared,
                                                    retention_days,
                                                    "pruned archived session data",
                                                );
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    workspace_id = %workspace.id.0,
                                                    retention_days,
                                                    "failed to prune old session data: {err:#}",
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            workspace_id = %workspace.id.0,
                                            "failed to open workspace store for pruning: {err:#}",
                                        );
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                retention_days,
                                "failed to list workspaces for pruning: {err:#}",
                            );
                        }
                    }
                    last_cleanup = Some(today);
                }
                tokio::time::sleep(Duration::from_secs(60 * 60)).await;
            }
        });
    }

    let agent_cfg = installer::load_agent_server_config(&data_root)
        .await
        .unwrap_or_default();

    match installer::ensure_claude_code_acp_ask_user_question_patched(&agent_cfg).await {
        Ok(true) => tracing::info!("patched claude-code-acp to enable AskUserQuestion over ACP"),
        Ok(false) => {}
        Err(e) => tracing::warn!("failed to patch claude-code-acp for AskUserQuestion: {e:#}"),
    }

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    let codex_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("codex")
            .map(|c| Tier1AcpAdapter::from_raw("codex", c.command.clone(), c.args.clone()))
            .unwrap_or_else(Tier1AcpAdapter::codex),
    );
    let claude_cmd = agent_cfg
        .providers
        .get("claude")
        .map(|c| (c.command.clone(), c.args.clone()));
    let gemini_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("gemini")
            .map(|c| Tier1AcpAdapter::from_raw("gemini", c.command.clone(), c.args.clone()))
            .unwrap_or_else(Tier1AcpAdapter::gemini),
    );
    let qwen_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("qwen")
            .map(|c| Tier1AcpAdapter::from_raw("qwen", c.command.clone(), c.args.clone()))
            .unwrap_or_else(|| {
                Tier1AcpAdapter::from_raw(
                    "qwen",
                    "qwen".to_string(),
                    vec!["--experimental-acp".to_string()],
                )
            }),
    );
    let opencode_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("opencode")
            .map(|c| Tier1AcpAdapter::from_raw("opencode", c.command.clone(), c.args.clone()))
            .unwrap_or_else(|| {
                Tier1AcpAdapter::from_raw(
                    "opencode",
                    "opencode".to_string(),
                    vec!["acp".to_string()],
                )
            }),
    );
    let mistral_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("mistral")
            .map(|c| Tier1AcpAdapter::from_raw("mistral", c.command.clone(), c.args.clone()))
            .unwrap_or_else(|| {
                Tier1AcpAdapter::from_raw("mistral", "vibe-acp".to_string(), vec![])
            }),
    );
    let goose_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("goose")
            .map(|c| Tier1AcpAdapter::from_raw("goose", c.command.clone(), c.args.clone()))
            .unwrap_or_else(|| {
                Tier1AcpAdapter::from_raw("goose", "goose".to_string(), vec!["acp".to_string()])
            }),
    );
    let kimi_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("kimi")
            .map(|c| Tier1AcpAdapter::from_raw("kimi", c.command.clone(), c.args.clone()))
            .unwrap_or_else(|| {
                Tier1AcpAdapter::from_raw("kimi", "kimi".to_string(), vec!["--acp".to_string()])
            }),
    );
    let auggie_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("auggie")
            .map(|c| Tier1AcpAdapter::from_raw("auggie", c.command.clone(), c.args.clone()))
            .unwrap_or_else(|| {
                Tier1AcpAdapter::from_raw("auggie", "auggie".to_string(), vec!["--acp".to_string()])
            }),
    );
    let cagent_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("cagent")
            .map(|c| Tier1AcpAdapter::from_raw("cagent", c.command.clone(), c.args.clone()))
            .unwrap_or_else(|| {
                let cfg = data_root
                    .join("providers")
                    .join("agent-servers")
                    .join("cagent")
                    .join("config.yaml")
                    .to_string_lossy()
                    .to_string();
                Tier1AcpAdapter::from_raw(
                    "cagent",
                    "cagent".to_string(),
                    vec!["acp".to_string(), cfg],
                )
            }),
    );

    providers.insert("codex".into(), codex_adapter.clone());
    providers.insert("gemini".into(), gemini_adapter.clone());
    providers.insert("qwen".into(), qwen_adapter.clone());
    providers.insert("opencode".into(), opencode_adapter.clone());
    providers.insert("mistral".into(), mistral_adapter.clone());
    providers.insert("goose".into(), goose_adapter.clone());
    providers.insert("kimi".into(), kimi_adapter.clone());
    providers.insert("auggie".into(), auggie_adapter.clone());
    providers.insert("cagent".into(), cagent_adapter.clone());

    if std::env::var("CTX_SHOW_FAKE_PROVIDER").ok().as_deref() == Some("1") {
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    }

    // Register additional harnesses as ACP adapters so they appear in /providers even if not installed.
    // These binaries are expected to support ACP over stdio.
    for (id, command, args) in [
        ("qwen", "qwen", vec!["--experimental-acp"]),
        ("opencode", "opencode", vec!["acp"]),
        ("openhands", "openhands", vec!["acp"]),
        ("goose", "goose", vec!["acp"]),
        ("mistral", "vibe-acp", vec![]),
        ("amp", "amp-acp", vec![]),
        ("droid", "droid-acp", vec![]),
        ("copilot", "copilot-cli-acp", vec![]),
        ("kiro", "kiro-acp", vec![]),
        ("rovo", "rovo-dev-acp", vec![]),
        ("cody", "cody-acp", vec![]),
        ("continue", "cn", vec!["acp"]),
        ("cline", "cline-acp", vec![]),
        ("swe-agent", "sweagent", vec!["acp"]),
    ] {
        let adapter: Arc<Tier1AcpAdapter> = Arc::new(
            agent_cfg
                .providers
                .get(id)
                .map(|c| Tier1AcpAdapter::from_raw(id, c.command.clone(), c.args.clone()))
                .unwrap_or_else(|| {
                    Tier1AcpAdapter::from_raw(
                        id,
                        command.to_string(),
                        args.into_iter().map(|s| s.to_string()).collect(),
                    )
                }),
        );
        providers.insert(id.to_string(), adapter);
    }

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let host = match local_addr.ip() {
        std::net::IpAddr::V4(ip) if ip.octets() == [0, 0, 0, 0] => "127.0.0.1".to_string(),
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => "::1".to_string(),
        ip => ip.to_string(),
    };
    let daemon_url = format!("http://{}:{}", host, local_addr.port());

    let mut auth = auth::load_or_init_daemon_auth(&data_root)?;
    let auth_token = Some(auth.token.clone());
    let auth_token_for_env = auth_token.clone();
    let prewarm_workdir = data_root.clone();

    let mut lsp_cfg = LspManagerConfig::default();
    let _ = installer::apply_managed_lsp_server_config(&data_root, &mut lsp_cfg).await;
    let _ = installer::apply_user_lsp_server_config(&data_root, &mut lsp_cfg).await;
    auth.daemon_url = Some(daemon_url.clone());
    auth::write_daemon_auth_file(&auth::daemon_auth_path(&data_root), &auth)?;

    let state = Arc::new(AppState::new_with_lsp_config(
        data_root,
        stores,
        providers,
        daemon_url.clone(),
        auth_token,
        lsp_cfg,
    ));
    state.web_sessions.clone().start_reaper().await;
    if let Err(err) = reconcile_running_turns(&state).await {
        tracing::warn!(err = %err, "failed to reconcile running turns on startup");
    }
    let settings = settings::load_settings(&state.data_root).await;
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = settings.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = settings
        .telemetry
        .as_ref()
        .map(|t| t.enabled)
        .unwrap_or(true);
    state
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    if let Err(err) = provider_restart::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider restart settings: {err:#}");
    }

    if let Err(err) = tool_cgroup::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply tool cgroup settings: {err:#}");
    }

    resource_telemetry::spawn_resource_telemetry(state.clone());
    provider_guard::spawn_provider_guard(state.clone());
    provider_restart::spawn_provider_restart(state.clone());
    provider_child_reclassifier::spawn_provider_child_reclassifier(state.clone());
    crate::merge_queue::spawn_merge_queue_runner(state.clone());
    provider_usage::spawn_provider_usage_poller(state.clone());

    // Reconnect managed mobile access tunnel on daemon start when enabled.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if state.auth_token.is_none() {
                return;
            }
            let cfg = match state.global_store().get_mobile_access_config().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("failed to read saved mobile access config: {e:#}");
                    return;
                }
            };
            let Some(cfg) = cfg else {
                return;
            };
            if !cfg.enabled {
                return;
            }

            let start_cfg = crate::mobile_tunnel::StartMobileTunnelConfig {
                relay_base_url: cfg.relay_base_url,
                tunnel_id: cfg.tunnel_id,
                tunnel_secret: cfg.tunnel_secret,
                public_base_url: cfg.public_base_url.trim_end_matches('/').to_string(),
                local_daemon_url: state.daemon_url.trim_end_matches('/').to_string(),
            };
            if let Err(e) = state.mobile_tunnel.start(start_cfg).await {
                tracing::warn!("failed to start saved mobile tunnel: {e:#}");
            }
        });
    }

    let codex_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("codex")
            .map(|c| {
                Tier1AcpAdapter::from_raw_with_ask_user_question(
                    "codex",
                    c.command.clone(),
                    c.args.clone(),
                    Arc::clone(&state.ask_user_question),
                )
            })
            .unwrap_or_else(|| {
                Tier1AcpAdapter::codex_with_ask_user_question(Arc::clone(&state.ask_user_question))
            }),
    );
    {
        let mut map = state.providers.lock().await;
        map.insert("codex".into(), codex_adapter.clone());
    }

    // Claude-only extension plumbing: AskUserQuestion is implemented via a Claude-specific ACP
    // extension method and should not be threaded into other providers.
    let claude_adapter: Arc<Tier1AcpAdapter> = Arc::new(match claude_cmd {
        Some((command, args)) => Tier1AcpAdapter::claude_from_raw_with_ask_user_question(
            command,
            args,
            Arc::clone(&state.ask_user_question),
        ),
        None => {
            Tier1AcpAdapter::claude_with_ask_user_question(Arc::clone(&state.ask_user_question))
        }
    });
    {
        let mut map = state.providers.lock().await;
        map.insert("claude".into(), claude_adapter.clone());
    }
    installer::refresh_provider_statuses(&state).await?;

    // Pinned ACP provider warming:
    // - Always keep these providers warm today: codex, gemini, claude
    // - For future providers, they start on first use and remain alive for daemon lifetime.
    let prewarm_ids: std::collections::HashSet<String> = std::env::var("CTX_ACP_PREWARM_PROVIDERS")
        .unwrap_or_else(|_| "codex,gemini,claude".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if !prewarm_ids.is_empty() {
        let mut base_env = HashMap::<String, String>::new();
        base_env.insert("CTX_DAEMON_URL".to_string(), daemon_url.clone());
        base_env.insert(
            "CTX_DATA_ROOT".to_string(),
            state.data_root.to_string_lossy().to_string(),
        );
        if let Some(token) = auth_token_for_env.clone() {
            base_env.insert("CTX_AUTH_TOKEN".to_string(), token);
        }
        for key in [
            "RUST_LOG",
            "RUST_BACKTRACE",
            "RUST_LOG_SPAN_EVENTS",
            "RUST_LIB_BACKTRACE",
        ] {
            if let Ok(value) = std::env::var(key) {
                base_env.insert(key.to_string(), value);
            }
        }
        if let Ok(value) = std::env::var("CTX_MCP_COMMAND") {
            base_env.insert("CTX_MCP_COMMAND".to_string(), value);
        }
        if let Ok(value) = std::env::var("CTX_MCP_DISABLED") {
            base_env.insert("CTX_MCP_DISABLED".to_string(), value);
        }

        for (id, adapter) in [
            ("codex", codex_adapter.clone()),
            ("gemini", gemini_adapter.clone()),
            ("claude", claude_adapter.clone()),
        ] {
            if !prewarm_ids.contains(id) {
                continue;
            }
            let workdir = prewarm_workdir.clone();
            let mut env = base_env.clone();
            if id == "codex" {
                if let Ok(extra) =
                    provider_accounts::codex_env_for_active_account(&state.data_root).await
                {
                    env.extend(extra);
                }
            }
            apply_acp_heap_profile_env(id, &mut env, &state.data_root);
            tokio::spawn(async move {
                let status = adapter.inspect().await;
                let ok = status.as_ref().is_ok_and(|s| {
                    s.installed && matches!(s.health, ctx_providers::adapters::ProviderHealth::Ok)
                });
                if !ok {
                    return;
                }
                if let Err(e) = adapter.prewarm(workdir, env).await {
                    tracing::warn!("failed to prewarm provider {id}: {e:#}");
                }
            });
        }
    }
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let app: Router = api::router(state);

    tracing::info!("ctx daemon listening on {daemon_url}");
    println!("{}", json!({"event":"listening","url": daemon_url}));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.recv().await;
        })
        .await?;
    Ok(())
}

pub async fn init_workspace(root: Option<String>) -> Result<()> {
    let root_path = root
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().context("getting current dir")?);
    let vcs = ctx_fs::vcs::driver_for_path(&root_path).await?;
    vcs.assert_repo(&root_path).await?;

    let context_dir = root_path.join(".ctx");
    let pack_dir = context_dir.join("ctx-pack");
    let tmp_dir = pack_dir.join("tmp");

    tokio::fs::create_dir_all(pack_dir.join("specs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("prompts")).await?;
    tokio::fs::create_dir_all(pack_dir.join("docs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("skills")).await?;
    tokio::fs::create_dir_all(&tmp_dir).await?;

    tokio::fs::create_dir_all(context_dir.join("exec-plans")).await?;

    let gitignore_path = root_path.join(".gitignore");
    let ignore_line = ".ctx/ctx-pack/tmp/";
    let mut gitignore = if gitignore_path.exists() {
        tokio::fs::read_to_string(&gitignore_path).await?
    } else {
        String::new()
    };
    if !gitignore.lines().any(|l| l.trim() == ignore_line) {
        if !gitignore.ends_with('\n') && !gitignore.is_empty() {
            gitignore.push('\n');
        }
        gitignore.push_str(ignore_line);
        gitignore.push('\n');
        tokio::fs::write(&gitignore_path, gitignore).await?;
    }

    Ok(())
}
