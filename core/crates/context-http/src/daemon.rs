use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use directories::BaseDirs;
use tokio::sync::{broadcast, mpsc, Mutex};

use context_core::ids::SessionId;
use context_core::models::{Session, SessionEvent};
use context_providers::adapters::ProviderAdapter;
use context_providers::adapters::ProviderStatus;
use context_providers::fake::FakeProviderAdapter;
use context_providers::tier1::Tier1AcpAdapter;
use context_store::Store;

use crate::api;
use crate::scheduler::{session_worker, SchedulerCommand};

pub struct AppState {
    pub data_root: PathBuf,
    pub store: Store,
    pub providers: HashMap<String, Arc<dyn ProviderAdapter>>,
    pub provider_statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub daemon_url: String,
    schedulers: Mutex<HashMap<SessionId, mpsc::Sender<SchedulerCommand>>>,
    broadcasters: Mutex<HashMap<SessionId, broadcast::Sender<SessionEvent>>>,
    running_sessions: Mutex<HashSet<SessionId>>,
}

impl AppState {
    pub fn new(
        data_root: PathBuf,
        store: Store,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
    ) -> Self {
        Self {
            data_root,
            store,
            providers,
            provider_statuses: Mutex::new(HashMap::new()),
            daemon_url,
            schedulers: Mutex::new(HashMap::new()),
            broadcasters: Mutex::new(HashMap::new()),
            running_sessions: Mutex::new(HashSet::new()),
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

}

pub async fn serve(bind: String, data_dir: Option<String>) -> Result<()> {
    let data_root = match data_dir {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("resolving home dir")?;
            base.home_dir().join(".context")
        }
    };
    tokio::fs::create_dir_all(&data_root).await?;
    let db_dir = data_root.join("db");
    tokio::fs::create_dir_all(&db_dir).await?;
    let db_path = db_dir.join("db.sqlite");
    let legacy_db_path = data_root.join("db.sqlite");
    if legacy_db_path.exists() && !db_path.exists() {
        tokio::fs::rename(&legacy_db_path, &db_path).await?;
    }
    let store = Store::open(&db_path).await?;

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    providers.insert("codex".into(), Arc::new(Tier1AcpAdapter::codex()));
    providers.insert("claude".into(), Arc::new(Tier1AcpAdapter::claude()));
    providers.insert("gemini".into(), Arc::new(Tier1AcpAdapter::gemini()));

    let daemon_url = format!("http://{}", bind.replace("0.0.0.0", "127.0.0.1"));
    let state = Arc::new(AppState::new(data_root, store, providers, daemon_url));
    {
        let mut statuses = HashMap::new();
        for (id, adapter) in state.providers.iter() {
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
    let app: Router = api::router(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("context daemon listening on http://{bind}");
    axum::serve(listener, app).await?;
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
