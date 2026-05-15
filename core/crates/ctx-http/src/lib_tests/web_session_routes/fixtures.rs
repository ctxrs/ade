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
        let state = test_daemon_for_test(data_dir.path(), daemon_secret.map(str::to_string)).await;

        Self {
            app: test_router(&state),
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
