use super::*;

pub(super) struct StreamingServer {
    pub(super) state: Arc<DaemonState>,
    pub(super) base: String,
    pub(super) addr: std::net::SocketAddr,
    pub(super) client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    git_repo: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
    _home: tempfile::TempDir,
    _home_guard: EnvVarGuard,
}

impl Drop for StreamingServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub(super) async fn start_streaming_server() -> StreamingServer {
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let home_guard = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        fake_provider_map(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    install_fake_provider_status(&state).await;

    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    StreamingServer {
        state,
        base: format!("http://{addr}"),
        addr,
        client: reqwest::Client::new(),
        server,
        git_repo,
        _data_dir: data_dir,
        _home: home,
        _home_guard: home_guard,
    }
}

pub(super) async fn create_default_task_session(
    harness: &StreamingServer,
) -> (
    ctx_core::models::Workspace,
    ctx_core::models::Task,
    ctx_core::models::Session,
) {
    let providers_res: Vec<ProviderStatus> = harness
        .client
        .get(format!("{}/api/providers", harness.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(providers_res.len(), 1);

    let workspace: ctx_core::models::Workspace = harness
        .client
        .post(format!("{}/api/workspaces", harness.base))
        .json(&json!({
            "root_path": harness.git_repo.path().to_string_lossy(),
            "name": "ws"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: ctx_core::models::Task = harness
        .client
        .post(format!(
            "{}/api/workspaces/{}/tasks",
            harness.base, workspace.id.0
        ))
        .json(&json!({
            "title": "t1",
            "default_session": fake_default_session_payload(),
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let sessions: Vec<ctx_core::models::Session> = harness
        .client
        .get(format!("{}/api/tasks/{}/sessions", harness.base, task.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = sessions
        .into_iter()
        .find(|session| Some(session.id) == task.primary_session_id)
        .expect("created task should list its default session");

    (workspace, task, session)
}

fn fake_provider_map() -> HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> {
    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    providers
}

async fn install_fake_provider_status(state: &Arc<DaemonState>) {
    let mut statuses = HashMap::new();
    statuses.insert(
        "fake".into(),
        ProviderStatus {
            provider_id: "fake".into(),
            installed: true,
            detected_path: None,
            version: Some("0.1.0".into()),
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: vec![],
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        },
    );
    state.providers.replace_provider_statuses(statuses).await;
}
