use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::Router;
use chrono::Utc;
use directories::BaseDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, mpsc, watch, Mutex, Notify};

use crate::buffers::BufferStore;
use crate::edit_plans::{EditPlan, EditPlanId};
use ctx_core::ids::{MessageId, SessionId, TaskId, TrackId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Session, SessionEvent, SessionEventType, SessionHeadDelta, SessionTurnStatus, TrackDiffSummary,
};
use ctx_lsp::Language as LspLanguage;
use ctx_lsp::{LspManager, LspManagerConfig};
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::adapters::ProviderStatus;
use ctx_providers::ask_user_question::AskUserQuestionBroker;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_providers::tier1::Tier1AcpAdapter;
use ctx_store::Store;

use crate::api;
use crate::installer;
use crate::installs::{InstallId, InstallProgressEvent, InstallState, InstallStateKind};
use crate::mobile_tunnel::MobileTunnelManager;
use crate::perf_telemetry::PerfTelemetry;
use crate::provider_guard;
use crate::resource_governance::{self, ResourceGovernanceRuntime};
use crate::resource_telemetry;
use crate::resource_utilization::ResourceSampler;
use crate::scheduler::{reconcile_turn_terminal_state, session_worker, SchedulerCommand};
use crate::settings;
use crate::telemetry::{Telemetry, TelemetryConfig};
use crate::terminals::TerminalManager;
use crate::web_sessions::WebSessionManager;
use crate::workspace_catchup::WorkspaceCatchupHub;

fn acquire_daemon_lock(data_root: &Path) -> Result<std::fs::File> {
    let path = data_root.join("daemon.lock");
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("opening daemon lockfile {}", path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.set_len(0);
            let _ = writeln!(file, "{}", std::process::id());
            let _ = file.sync_all();
            Ok(file)
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            anyhow::bail!("ctx daemon already running (lockfile {})", path.display())
        }
        Err(e) => Err(e).with_context(|| format!("locking daemon lockfile {}", path.display())),
    }
}

const DAEMON_AUTH_FILENAME: &str = "daemon_auth.json";

#[derive(Debug, Serialize, Deserialize)]
struct DaemonAuthFile {
    token: String,
    #[serde(default)]
    daemon_url: Option<String>,
}

fn daemon_auth_path(data_root: &Path) -> PathBuf {
    data_root.join(DAEMON_AUTH_FILENAME)
}

fn read_daemon_auth_file(path: &Path) -> Result<Option<DaemonAuthFile>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let auth: DaemonAuthFile = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing daemon auth file {}", path.display()))?;
            if auth.token.trim().is_empty() {
                anyhow::bail!("daemon auth file {} contains empty token", path.display());
            }
            Ok(Some(auth))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => {
            Err(err).with_context(|| format!("reading daemon auth file {}", path.display()))
        }
    }
}

fn write_daemon_auth_file(path: &Path, auth: &DaemonAuthFile) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(auth)?;
    std::fs::write(&tmp, bytes)?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
    Ok(())
}

fn load_or_init_daemon_auth(data_root: &Path) -> Result<DaemonAuthFile> {
    let path = daemon_auth_path(data_root);
    if let Some(auth) = read_daemon_auth_file(&path)? {
        return Ok(auth);
    }
    let auth = DaemonAuthFile {
        token: uuid::Uuid::new_v4().to_string(),
        daemon_url: None,
    };
    write_daemon_auth_file(&path, &auth)?;
    Ok(auth)
}

pub struct AppState {
    pub data_root: PathBuf,
    pub store: Store,
    pub providers: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub provider_statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub provider_matrix_cache: Mutex<crate::provider_matrix::ProviderMatrixCache>,
    pub provider_options_cache: Mutex<HashMap<String, CachedProviderOptions>>,
    pub provider_verify_cache: Mutex<HashMap<String, CachedProviderVerify>>,
    pub diff_summary_cache: Mutex<HashMap<TrackId, CachedDiffSummary>>,
    pub file_completions_cache: Mutex<HashMap<WorktreeId, CachedFileCompletions>>,
    pub workspace_file_completions_cache: Mutex<HashMap<WorkspaceId, CachedFileCompletions>>,
    pub daemon_url: String,
    pub auth_token: Option<String>,
    pub lsp_cfg: LspManagerConfig,
    pub lsp: Arc<LspManager>,
    pub lsp_edit_plans_enabled: bool,
    pub buffers: BufferStore,
    pub ask_user_question: Arc<AskUserQuestionBroker>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub telemetry: Telemetry,
    pub perf_telemetry: PerfTelemetry,
    pub resource_governance: Mutex<ResourceGovernanceRuntime>,
    pub provider_guard: Mutex<provider_guard::ProviderGuardRuntime>,
    pub resource_sampler: Mutex<ResourceSampler>,
    pub workspace_catchup: WorkspaceCatchupHub,
    pub terminals: TerminalManager,
    pub mobile_tunnel: MobileTunnelManager,
    pub web_sessions: Arc<WebSessionManager>,
    pub merge_queue_notify: Arc<Notify>,
    schedulers: Mutex<HashMap<SessionId, mpsc::Sender<SchedulerCommand>>>,
    broadcasters: Mutex<HashMap<SessionId, broadcast::Sender<SessionEvent>>>,
    session_event_heads: Mutex<HashMap<SessionId, watch::Sender<i64>>>,
    global_broadcaster: broadcast::Sender<SessionEvent>,
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

#[derive(Clone)]
pub struct CachedDiffSummary {
    pub cached_at: Instant,
    pub summary: Option<TrackDiffSummary>,
    pub too_large: bool,
}

impl AppState {
    pub fn new(
        data_root: PathBuf,
        store: Store,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
    ) -> Self {
        Self::new_with_lsp_config(
            data_root,
            store,
            providers,
            daemon_url,
            auth_token,
            LspManagerConfig::default(),
        )
    }

    pub fn new_with_lsp_config(
        data_root: PathBuf,
        store: Store,
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
            store,
            providers,
            daemon_url,
            auth_token,
            lsp_cfg,
            lsp_edit_plans_enabled,
        )
    }

    pub fn new_with_lsp_config_and_flags(
        data_root: PathBuf,
        store: Store,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
        lsp_cfg: LspManagerConfig,
        lsp_edit_plans_enabled: bool,
    ) -> Self {
        let edit_plans_dir = edit_plans_dir(&data_root);
        if let Err(e) = std::fs::create_dir_all(&edit_plans_dir) {
            tracing::warn!(
                "failed to create edit plans dir {}: {e}",
                edit_plans_dir.to_string_lossy()
            );
        }
        let edit_plans = load_edit_plans_from_disk(&data_root);

        let (shutdown_tx, _) = broadcast::channel(8);
        let (global_broadcaster, _) = broadcast::channel(2048);
        let (lsp_diag_broadcaster, _) = broadcast::channel(2048);
        let ask_user_question = Arc::new(AskUserQuestionBroker::new());
        let lsp = Arc::new(LspManager::new(lsp_cfg.clone()));
        let telemetry = Telemetry::new(data_root.clone());
        let perf_telemetry = PerfTelemetry::new(data_root.clone());
        let workspace_catchup = WorkspaceCatchupHub::new();
        let web_sessions = Arc::new(WebSessionManager::new());
        let merge_queue_notify = Arc::new(Notify::new());
        Self {
            data_root,
            store,
            providers: Mutex::new(providers),
            provider_statuses: Mutex::new(HashMap::new()),
            provider_matrix_cache: Mutex::new(
                crate::provider_matrix::ProviderMatrixCache::default(),
            ),
            provider_options_cache: Mutex::new(HashMap::new()),
            provider_verify_cache: Mutex::new(HashMap::new()),
            diff_summary_cache: Mutex::new(HashMap::new()),
            file_completions_cache: Mutex::new(HashMap::new()),
            workspace_file_completions_cache: Mutex::new(HashMap::new()),
            daemon_url,
            auth_token,
            lsp_cfg,
            lsp,
            lsp_edit_plans_enabled,
            buffers: BufferStore::default(),
            ask_user_question,
            shutdown_tx,
            telemetry,
            perf_telemetry,
            resource_governance: Mutex::new(ResourceGovernanceRuntime::default()),
            provider_guard: Mutex::new(provider_guard::ProviderGuardRuntime::default()),
            resource_sampler: Mutex::new(ResourceSampler::new()),
            workspace_catchup,
            terminals: TerminalManager::default(),
            mobile_tunnel: MobileTunnelManager::default(),
            web_sessions,
            merge_queue_notify,
            schedulers: Mutex::new(HashMap::new()),
            broadcasters: Mutex::new(HashMap::new()),
            session_event_heads: Mutex::new(HashMap::new()),
            global_broadcaster,
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
        edit_plans_dir(&self.data_root)
    }

    pub fn persist_edit_plan(&self, plan: &EditPlan) {
        if let Err(e) = persist_edit_plan_to_disk(&self.data_root, plan) {
            tracing::warn!("failed to persist edit plan {}: {e}", plan.id.0);
        }
    }

    pub fn delete_edit_plan_file(&self, plan_id: EditPlanId) {
        if let Err(e) = delete_edit_plan_file(&self.data_root, plan_id) {
            tracing::warn!("failed to delete edit plan {} file: {e}", plan_id.0);
        }
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

    pub fn global_broadcaster(&self) -> broadcast::Sender<SessionEvent> {
        self.global_broadcaster.clone()
    }

    pub fn lsp_diag_broadcaster(&self) -> broadcast::Sender<serde_json::Value> {
        self.lsp_diag_broadcaster.clone()
    }

    pub async fn publish_event(&self, event: SessionEvent) {
        let tx = self.get_broadcaster(event.session_id).await;
        let _ = tx.send(event.clone());
        let _ = self.global_broadcaster.send(event.clone());
        let mut map = self.session_event_heads.lock().await;
        let sender = map.entry(event.session_id).or_insert_with(|| {
            let (tx, _rx) = watch::channel::<i64>(0);
            tx
        });
        let _ = sender.send(event.seq);
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        let mut cache = self.session_meta_cache.lock().await;
        cache.insert(session.id, session.clone());
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

    async fn session_meta(&self, session_id: SessionId) -> Option<Session> {
        {
            let cache = self.session_meta_cache.lock().await;
            if let Some(session) = cache.get(&session_id) {
                return Some(session.clone());
            }
        }
        let session = self.store.get_session(session_id).await.ok().flatten()?;
        let mut cache = self.session_meta_cache.lock().await;
        cache.insert(session_id, session.clone());
        Some(session)
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> Result<()> {
        if let Some(summary) = self
            .store
            .get_workspace_catchup_task_summary(task_id)
            .await?
        {
            let workspace_id = summary.task.workspace_id;
            self.workspace_catchup
                .publish_task_upsert(workspace_id, summary)
                .await;
        }
        Ok(())
    }

    pub async fn emit_workspace_track_upsert(&self, track_id: TrackId) -> Result<()> {
        if let Some(summary) = self
            .store
            .get_workspace_catchup_track_summary(track_id)
            .await?
        {
            let workspace_id = summary.track.workspace_id;
            self.workspace_catchup
                .publish_track_upsert(workspace_id, summary)
                .await;
        }
        Ok(())
    }

    pub async fn emit_workspace_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        self.workspace_catchup
            .publish_task_delete(workspace_id, task_id)
            .await;
    }

    pub fn start_workspace_catchup_listener(self: &Arc<Self>) {
        let state = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut rx = match state.upgrade() {
                Some(state) => state.global_broadcaster.subscribe(),
                None => return,
            };
            loop {
                let event = match rx.recv().await {
                    Ok(event) => event,
                    Err(_) => break,
                };
                let Some(state) = state.upgrade() else {
                    break;
                };

                let Some(session) = state.session_meta(event.session_id).await else {
                    continue;
                };
                let update_task = matches!(
                    event.event_type,
                    SessionEventType::UserMessage
                        | SessionEventType::AssistantMessageInserted
                        | SessionEventType::AssistantComplete
                        | SessionEventType::Done
                        | SessionEventType::TurnInterrupted
                        | SessionEventType::Error
                );
                if update_task {
                    let _ = state.emit_workspace_task_upsert(session.task_id).await;
                }

                let message = if matches!(
                    event.event_type,
                    SessionEventType::UserMessage | SessionEventType::AssistantMessageInserted
                ) {
                    let message_id = event
                        .payload_json
                        .get("message_id")
                        .and_then(|v| v.as_str())
                        .and_then(|id| uuid::Uuid::parse_str(id).ok())
                        .map(MessageId);
                    if let Some(message_id) = message_id {
                        state.store.get_message(message_id).await.ok().flatten()
                    } else {
                        None
                    }
                } else {
                    None
                };

                let turn = if matches!(event.event_type, SessionEventType::UserMessage) {
                    match event.turn_id {
                        Some(turn_id) => state
                            .store
                            .get_session_turn(event.session_id, turn_id)
                            .await
                            .ok()
                            .flatten(),
                        None => None,
                    }
                } else {
                    None
                };

                let delta = SessionHeadDelta {
                    session_id: event.session_id,
                    last_event_seq: event.seq,
                    event: Some(event.clone()),
                    turn,
                    message,
                };
                state
                    .workspace_catchup
                    .publish_session_head_delta(session.workspace_id, delta)
                    .await;
            }
        });
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

    pub async fn ensure_scheduler(
        self: &Arc<Self>,
        session: Session,
    ) -> mpsc::Sender<SchedulerCommand> {
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

async fn reconcile_running_turns(state: &Arc<AppState>) -> Result<()> {
    let running_turns = state
        .store
        .list_session_turns_by_statuses(&[SessionTurnStatus::Running])
        .await?;

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

fn edit_plans_dir(data_root: &Path) -> PathBuf {
    data_root.join("edit_plans")
}

fn edit_plan_path(data_root: &Path, plan_id: EditPlanId) -> PathBuf {
    edit_plans_dir(data_root).join(format!("{}.json", plan_id.0))
}

fn load_edit_plans_from_disk(data_root: &Path) -> HashMap<EditPlanId, EditPlan> {
    let mut out = HashMap::new();
    let dir = edit_plans_dir(data_root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    "failed to read edit plan file {}: {e}",
                    path.to_string_lossy()
                );
                continue;
            }
        };
        match serde_json::from_slice::<EditPlan>(&bytes) {
            Ok(plan) => {
                out.insert(plan.id, plan);
            }
            Err(e) => {
                tracing::warn!(
                    "failed to parse edit plan file {}: {e}",
                    path.to_string_lossy()
                );
            }
        }
    }
    out
}

fn persist_edit_plan_to_disk(data_root: &Path, plan: &EditPlan) -> anyhow::Result<()> {
    let path = edit_plan_path(data_root, plan.id);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(plan)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn delete_edit_plan_file(data_root: &Path, plan_id: EditPlanId) -> anyhow::Result<()> {
    let path = edit_plan_path(data_root, plan_id);
    let _ = std::fs::remove_file(&path);
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

    let _daemon_lock = acquire_daemon_lock(&data_root)?;

    let db_dir = data_root.join("db");
    tokio::fs::create_dir_all(&db_dir).await?;
    let db_path = db_dir.join("db.sqlite");
    let legacy_db_path = data_root.join("db.sqlite");
    if legacy_db_path.exists() && !db_path.exists() {
        tokio::fs::rename(&legacy_db_path, &db_path).await?;
    }
    let store = Store::open(&db_path).await?;

    // Hard-coded retention policy (no config surface yet):
    // - Keep tool summaries and final thoughts for 30 days.
    // - Do not retain thought chunk events (handled at ingestion time).
    const SESSION_RETENTION_DAYS: u64 = 30;
    {
        let store = store.clone();
        tokio::spawn(async move {
            let mut last_cleanup = None::<String>;
            loop {
                let today = Utc::now().format("%Y-%m-%d").to_string();
                if last_cleanup.as_deref() != Some(&today) {
                    match store
                        .prune_session_data_older_than_days(SESSION_RETENTION_DAYS)
                        .await
                    {
                        Ok(stats) => {
                            tracing::info!(
                                tool_summaries_deleted = stats.tool_summaries_deleted,
                                turn_thoughts_cleared = stats.turn_thoughts_cleared,
                                retention_days = SESSION_RETENTION_DAYS,
                                "pruned old session data",
                            );
                        }
                        Err(err) => {
                            tracing::warn!(
                                retention_days = SESSION_RETENTION_DAYS,
                                "failed to prune old session data: {err:#}",
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

    let mut auth = load_or_init_daemon_auth(&data_root)?;
    let auth_token = Some(auth.token.clone());
    let auth_token_for_env = auth_token.clone();
    let prewarm_workdir = data_root.clone();

    let mut lsp_cfg = LspManagerConfig::default();
    let _ = installer::apply_managed_lsp_server_config(&data_root, &mut lsp_cfg).await;
    let _ = installer::apply_user_lsp_server_config(&data_root, &mut lsp_cfg).await;
    auth.daemon_url = Some(daemon_url.clone());
    write_daemon_auth_file(&daemon_auth_path(&data_root), &auth)?;

    let state = Arc::new(AppState::new_with_lsp_config(
        data_root,
        store,
        providers,
        daemon_url.clone(),
        auth_token,
        lsp_cfg,
    ));
    state.start_workspace_catchup_listener();
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

    resource_telemetry::spawn_resource_telemetry(state.clone());
    provider_guard::spawn_provider_guard(state.clone());
    crate::merge_queue::spawn_merge_queue_runner(state.clone());

    // Reconnect managed mobile access tunnel on daemon start when enabled.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if state.auth_token.is_none() {
                return;
            }
            let cfg = match state.store.get_mobile_access_config().await {
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
        if let Some(token) = auth_token_for_env.clone() {
            base_env.insert("CTX_AUTH_TOKEN".to_string(), token);
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
            let env = base_env.clone();
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
    ctx_fs::git::assert_git_repo(&root_path).await?;

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
