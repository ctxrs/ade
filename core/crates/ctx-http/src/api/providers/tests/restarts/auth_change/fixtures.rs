use super::*;

pub(super) struct ProviderRestartFixture {
    pub(super) _temp: tempfile::TempDir,
    pub(super) daemon: TestDaemon,
}

pub(super) async fn fixture_with_adapter(
    adapter: Arc<dyn ProviderAdapter>,
) -> ProviderRestartFixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let daemon = TestDaemon::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::from([("codex".to_string(), adapter)]),
        "http://127.0.0.1:4310".to_string(),
        None,
    );

    ProviderRestartFixture {
        _temp: temp,
        daemon,
    }
}

pub(super) async fn seed_options_probe_cache(
    daemon: &TestDaemon,
    key: &str,
    provider_id: &str,
    probe_ok: bool,
) {
    daemon
        .seed_provider_options_probe_cache_for_test(key, provider_id, probe_ok)
        .await;
}

pub(super) async fn seed_verify_cache_status(daemon: &TestDaemon, key: &str, status: &str) {
    daemon
        .seed_provider_verify_cache_status_for_test(key, status)
        .await;
}
