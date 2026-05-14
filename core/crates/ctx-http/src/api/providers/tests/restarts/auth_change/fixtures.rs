use super::*;

pub(super) struct ProviderRestartFixture {
    pub(super) _temp: tempfile::TempDir,
    pub(super) state: Arc<DaemonState>,
}

pub(super) async fn fixture_with_adapter(
    adapter: Arc<dyn ProviderAdapter>,
) -> ProviderRestartFixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(DaemonState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::from([("codex".to_string(), adapter)]),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));

    ProviderRestartFixture { _temp: temp, state }
}

pub(super) async fn insert_options_cache(
    state: &Arc<DaemonState>,
    key: &str,
    value: serde_json::Value,
) {
    state
        .test_with_provider_options_cache(|cache| {
            cache.insert(
                key.to_string(),
                CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value,
                },
            );
        })
        .await;
}

pub(super) async fn insert_verify_cache(
    state: &Arc<DaemonState>,
    key: &str,
    value: serde_json::Value,
) {
    state
        .test_with_provider_verify_cache(|cache| {
            cache.insert(
                key.to_string(),
                CachedProviderVerify {
                    cached_at: std::time::Instant::now(),
                    value,
                },
            );
        })
        .await;
}
