use super::*;

pub(super) struct WebSessionRouteFixture {
    app: axum::Router,
    _data_dir: tempfile::TempDir,
    _home: EnvVarGuard,
    _home_dir: tempfile::TempDir,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

impl WebSessionRouteFixture {
    pub(super) async fn new(daemon_secret: Option<&str>) -> Self {
        let serial = home_env_test_lock().lock().await;
        let home_dir = tempfile::tempdir().unwrap();
        let home = EnvVarGuard::set("HOME", &home_dir.path().to_string_lossy());

        let data_dir = tempfile::tempdir().unwrap();
        let stores = StoreManager::open(data_dir.path()).await.unwrap();
        let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
            HashMap::new();
        let state = Arc::new(AppState::new(
            data_dir.path().to_path_buf(),
            stores,
            providers,
            "http://127.0.0.1:4399".to_string(),
            daemon_secret.map(str::to_string),
        ));

        Self {
            app: api::router(state),
            _data_dir: data_dir,
            _home: home,
            _home_dir: home_dir,
            _serial: serial,
        }
    }

    pub(super) fn app(&self) -> axum::Router {
        self.app.clone()
    }
}
