use super::*;

#[tokio::test]
async fn select_provider_harness_source_invalidates_only_matching_provider_probe_caches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));

    state.providers.options_cache.lock().await.insert(
        "ws-a/host/codex".to_string(),
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
        },
    );
    state.providers.options_cache.lock().await.insert(
        "ws-b/container/claude-crp".to_string(),
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "provider_id": "claude-crp", "probe_ok": true }),
        },
    );
    state.providers.verify_cache.lock().await.insert(
        "ws-a/host/codex".to_string(),
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "status": "error" }),
        },
    );
    state.providers.verify_cache.lock().await.insert(
        "ws-b/container/claude-crp".to_string(),
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({ "status": "ok" }),
        },
    );

    let Json(config) = select_provider_harness_source(
        State(Arc::clone(&state)),
        Path("codex".to_string()),
        Json(SelectHarnessSourceReq {
            source_kind: HarnessSourceKind::Subscription,
            endpoint_id: None,
        }),
    )
    .await
    .expect("select provider harness source");

    assert_eq!(config.provider_id, "codex");
    assert_eq!(config.selected_source_kind, HarnessSourceKind::Subscription);

    let options_cache = state.providers.options_cache.lock().await;
    assert!(!options_cache.contains_key("ws-a/host/codex"));
    assert!(options_cache.contains_key("ws-b/container/claude-crp"));
    drop(options_cache);

    let verify_cache = state.providers.verify_cache.lock().await;
    assert!(!verify_cache.contains_key("ws-a/host/codex"));
    assert!(verify_cache.contains_key("ws-b/container/claude-crp"));
}
