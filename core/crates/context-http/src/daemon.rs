use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use chrono::Utc;
use directories::BaseDirs;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, Mutex};

use context_core::ids::SessionId;
use context_core::models::{Session, SessionEvent};
use context_providers::adapters::ProviderAdapter;
use context_providers::adapters::ProviderStatus;
use context_providers::fake::FakeProviderAdapter;
use context_providers::tier1::Tier1AcpAdapter;
use context_store::Store;

use crate::api;
use crate::installs::{InstallId, InstallProgressEvent, InstallState, InstallStateKind};
use crate::installer;
use crate::scheduler::{session_worker, SchedulerCommand};

pub struct AppState {
    pub data_root: PathBuf,
    pub store: Store,
    pub providers: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    pub provider_statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub daemon_url: String,
    pub auth_token: Option<String>,
    pub shutdown_tx: broadcast::Sender<()>,
    schedulers: Mutex<HashMap<SessionId, mpsc::Sender<SchedulerCommand>>>,
    broadcasters: Mutex<HashMap<SessionId, broadcast::Sender<SessionEvent>>>,
    running_sessions: Mutex<HashSet<SessionId>>,
    installs: Mutex<HashMap<InstallId, InstallState>>,
}

impl AppState {
    pub fn new(
        data_root: PathBuf,
        store: Store,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
    ) -> Self {
        let (shutdown_tx, _) = broadcast::channel(8);
        Self {
            data_root,
            store,
            providers: Mutex::new(providers),
            provider_statuses: Mutex::new(HashMap::new()),
            daemon_url,
            auth_token,
            shutdown_tx,
            schedulers: Mutex::new(HashMap::new()),
            broadcasters: Mutex::new(HashMap::new()),
            running_sessions: Mutex::new(HashSet::new()),
            installs: Mutex::new(HashMap::new()),
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
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    providers.insert(
        "codex".into(),
        Arc::new(
            agent_cfg
                .providers
                .get("codex")
                .map(|c| Tier1AcpAdapter::from_raw("codex", c.command.clone(), c.args.clone()))
                .unwrap_or_else(Tier1AcpAdapter::codex),
        ),
    );
    providers.insert(
        "claude".into(),
        Arc::new(
            agent_cfg
                .providers
                .get("claude")
                .map(|c| Tier1AcpAdapter::from_raw("claude", c.command.clone(), c.args.clone()))
                .unwrap_or_else(Tier1AcpAdapter::claude),
        ),
    );
    providers.insert(
        "gemini".into(),
        Arc::new(
            agent_cfg
                .providers
                .get("gemini")
                .map(|c| Tier1AcpAdapter::from_raw("gemini", c.command.clone(), c.args.clone()))
                .unwrap_or_else(Tier1AcpAdapter::gemini),
        ),
    );

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

    let state = Arc::new(AppState::new(
        data_root,
        store,
        providers,
        daemon_url.clone(),
        auth_token,
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
