use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::Router;
use chrono::Utc;
use directories::BaseDirs;
use fs2::FileExt;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, Mutex};

use context_core::ids::{SessionId, WorkspaceId, WorktreeId};
use context_core::models::{Session, SessionEvent};
use context_providers::adapters::ProviderAdapter;
use context_providers::adapters::ProviderStatus;
use context_providers::tier1::Tier1AcpAdapter;
use context_store::Store;
use context_lsp::{LspManager, LspManagerConfig};
use crate::edit_plans::{EditPlan, EditPlanId};
use crate::buffers::BufferStore;
use context_lsp::Language as LspLanguage;

use crate::api;
use crate::installs::{InstallId, InstallProgressEvent, InstallState, InstallStateKind};
use crate::installer;
use crate::scheduler::{session_worker, SchedulerCommand};

fn acquire_daemon_lock(data_root: &Path) -> Result<std::fs::File> {
    let path = data_root.join("daemon.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening daemon lockfile {}", path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => Ok(file),
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            anyhow::bail!("Context daemon already running (lockfile {})", path.display())
        }
        Err(e) => Err(e).with_context(|| format!("locking daemon lockfile {}", path.display())),
    }
}

pub struct AppState {
    pub data_root: PathBuf,
    pub store: Store,
    pub providers: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub provider_statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub provider_options_cache: Mutex<HashMap<String, CachedProviderOptions>>,
    pub file_completions_cache: Mutex<HashMap<WorktreeId, CachedFileCompletions>>,
    pub workspace_file_completions_cache: Mutex<HashMap<WorkspaceId, CachedFileCompletions>>,
    pub daemon_url: String,
    pub auth_token: Option<String>,
    pub lsp_cfg: LspManagerConfig,
    pub lsp: Arc<LspManager>,
    pub lsp_edit_plans_enabled: bool,
    pub buffers: BufferStore,
    pub shutdown_tx: broadcast::Sender<()>,
    schedulers: Mutex<HashMap<SessionId, mpsc::Sender<SchedulerCommand>>>,
    broadcasters: Mutex<HashMap<SessionId, broadcast::Sender<SessionEvent>>>,
    global_broadcaster: broadcast::Sender<SessionEvent>,
    lsp_diag_broadcaster: broadcast::Sender<serde_json::Value>,
    lsp_diag_forwarders: Mutex<HashSet<String>>,
    running_sessions: Mutex<HashSet<SessionId>>,
    installs: Mutex<HashMap<InstallId, InstallState>>,
    pub edit_plans: Mutex<HashMap<EditPlanId, EditPlan>>,
}

pub struct CachedProviderOptions {
    pub cached_at: Instant,
    pub value: serde_json::Value,
}

pub struct CachedFileCompletions {
    pub cached_at: Instant,
    pub files: Arc<Vec<String>>,
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
        let lsp_edit_plans_enabled = std::env::var("CONTEXT_LSP_EDITPLANS_ENABLED")
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
        let lsp = Arc::new(LspManager::new(lsp_cfg.clone()));
        Self {
            data_root,
            store,
            providers: Mutex::new(providers),
            provider_statuses: Mutex::new(HashMap::new()),
            provider_options_cache: Mutex::new(HashMap::new()),
            file_completions_cache: Mutex::new(HashMap::new()),
            workspace_file_completions_cache: Mutex::new(HashMap::new()),
            daemon_url,
            auth_token,
            lsp_cfg,
            lsp,
            lsp_edit_plans_enabled,
            buffers: BufferStore::default(),
            shutdown_tx,
            schedulers: Mutex::new(HashMap::new()),
            broadcasters: Mutex::new(HashMap::new()),
            global_broadcaster,
            lsp_diag_broadcaster,
            lsp_diag_forwarders: Mutex::new(HashSet::new()),
            running_sessions: Mutex::new(HashSet::new()),
            installs: Mutex::new(HashMap::new()),
            edit_plans: Mutex::new(edit_plans),
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

    pub fn global_broadcaster(&self) -> broadcast::Sender<SessionEvent> {
        self.global_broadcaster.clone()
    }

    pub fn lsp_diag_broadcaster(&self) -> broadcast::Sender<serde_json::Value> {
        self.lsp_diag_broadcaster.clone()
    }

    pub async fn publish_event(&self, event: SessionEvent) {
        let tx = self.get_broadcaster(event.session_id).await;
        let _ = tx.send(event.clone());
        let _ = self.global_broadcaster.send(event);
    }

    pub async fn ensure_lsp_diagnostics_forwarder(self: &Arc<Self>, root: PathBuf, lang: LspLanguage) {
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
            let mut rx = match state.lsp.subscribe_diagnostics_for_language(&root, lang).await {
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
                let Ok(abs_path) = url.to_file_path() else { continue };

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

    pub async fn get_install_info(&self, install_id: InstallId) -> Option<crate::installs::InstallInfo> {
        self.installs
            .lock()
            .await
            .get(&install_id)
            .map(|s| s.info(install_id))
    }

    pub async fn get_install_events(&self, install_id: InstallId) -> Option<Vec<InstallProgressEvent>> {
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
                tracing::warn!("failed to read edit plan file {}: {e}", path.to_string_lossy());
                continue;
            }
        };
        match serde_json::from_slice::<EditPlan>(&bytes) {
            Ok(plan) => {
                out.insert(plan.id, plan);
            }
            Err(e) => {
                tracing::warn!("failed to parse edit plan file {}: {e}", path.to_string_lossy());
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

pub async fn serve(
    bind: String,
    data_dir: Option<String>,
    auth_token: Option<String>,
    auth_disabled: bool,
) -> Result<()> {
    let data_root = match data_dir {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("resolving home dir")?;
            base.home_dir().join(".context")
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

    let agent_cfg = installer::load_agent_server_config(&data_root)
        .await
        .unwrap_or_default();

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    let codex_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("codex")
            .map(|c| Tier1AcpAdapter::from_raw("codex", c.command.clone(), c.args.clone()))
            .unwrap_or_else(Tier1AcpAdapter::codex),
    );
    let claude_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("claude")
            .map(|c| Tier1AcpAdapter::from_raw("claude", c.command.clone(), c.args.clone()))
            .unwrap_or_else(Tier1AcpAdapter::claude),
    );
    let gemini_adapter: Arc<Tier1AcpAdapter> = Arc::new(
        agent_cfg
            .providers
            .get("gemini")
            .map(|c| Tier1AcpAdapter::from_raw("gemini", c.command.clone(), c.args.clone()))
            .unwrap_or_else(Tier1AcpAdapter::gemini),
    );

    providers.insert("codex".into(), codex_adapter.clone());
    providers.insert("claude".into(), claude_adapter.clone());
    providers.insert("gemini".into(), gemini_adapter.clone());

    // Register additional harnesses as ACP adapters so they appear in /providers even if not installed.
    // These binaries are expected to support ACP over stdio.
    for (id, command, args) in [
        ("qwen", "qwen", vec!["--experimental-acp"]),
        ("opencode", "opencode", vec!["acp"]),
        ("goose", "goose", vec!["acp"]),
        ("mistral", "vibe-acp", vec![]),
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

    let auth_token = if auth_disabled {
        None
    } else {
        auth_token.or_else(|| std::env::var("CONTEXT_DESKTOP_TOKEN").ok())
    };
    let auth_token_for_env = auth_token.clone();
    let prewarm_workdir = data_root.clone();

    let mut lsp_cfg = LspManagerConfig::default();
    let _ = installer::apply_managed_lsp_server_config(&data_root, &mut lsp_cfg).await;
    let _ = installer::apply_user_lsp_server_config(&data_root, &mut lsp_cfg).await;
    let state = Arc::new(AppState::new_with_lsp_config(
        data_root,
        store,
        providers,
        daemon_url.clone(),
        auth_token,
        lsp_cfg,
    ));
    {
        let mut statuses = HashMap::new();
        let map = state.providers.lock().await;
        for (id, adapter) in map.iter() {
            match adapter.inspect().await {
                Ok(status) => {
                    statuses.insert(id.clone(), status);
                }
                Err(e) => {
                    statuses.insert(
                        id.clone(),
                        ProviderStatus {
                            provider_id: id.clone(),
                            installed: false,
                            detected_path: None,
                            version: None,
                            capabilities: None,
                            health: context_providers::adapters::ProviderHealth::Error,
                            diagnostics: vec![e.to_string()],
                            details: HashMap::new(),
                        },
                    );
                }
            }
        }
        *state.provider_statuses.lock().await = statuses;
    }

    // Pinned ACP provider warming:
    // - Always keep these providers warm today: codex, gemini, claude
    // - For future providers, they start on first use and remain alive for daemon lifetime.
    let prewarm_ids: std::collections::HashSet<String> = std::env::var("CONTEXT_ACP_PREWARM_PROVIDERS")
        .unwrap_or_else(|_| "codex,gemini,claude".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if !prewarm_ids.is_empty() {
        let mut base_env = HashMap::<String, String>::new();
        base_env.insert("CONTEXT_DAEMON_URL".to_string(), daemon_url.clone());
        if let Some(token) = auth_token_for_env.clone() {
            base_env.insert("CONTEXT_AUTH_TOKEN".to_string(), token);
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
                let ok = status
                    .as_ref()
                    .is_ok_and(|s| s.installed && matches!(s.health, context_providers::adapters::ProviderHealth::Ok));
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

    tracing::info!("context daemon listening on {daemon_url}");
    println!(
        "{}",
        json!({"event":"listening","url": daemon_url}).to_string()
    );
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
    context_fs::git::assert_git_repo(&root_path).await?;

    let context_dir = root_path.join(".context");
    let pack_dir = context_dir.join("context-pack");
    let tmp_dir = pack_dir.join("tmp");

    tokio::fs::create_dir_all(pack_dir.join("specs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("prompts")).await?;
    tokio::fs::create_dir_all(pack_dir.join("docs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("skills")).await?;
    tokio::fs::create_dir_all(&tmp_dir).await?;
    tokio::fs::create_dir_all(context_dir.join("exec-plans")).await?;

    let gitignore_path = root_path.join(".gitignore");
    let ignore_line = ".context/context-pack/tmp/";
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
