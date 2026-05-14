use super::*;
use ctx_provider_runtime::{CachedProviderOptions, CachedProviderVerify};

async fn insert_options_cache(state: &Arc<DaemonState>, key: &str, value: serde_json::Value) {
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

async fn insert_verify_cache(state: &Arc<DaemonState>, key: &str, value: serde_json::Value) {
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

#[tokio::test]
async fn select_provider_harness_source_invalidates_only_matching_provider_probe_caches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(DaemonState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));

    insert_options_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
    )
    .await;
    insert_options_cache(
        &state,
        "ws-b/container/claude-crp",
        serde_json::json!({ "provider_id": "claude-crp", "probe_ok": true }),
    )
    .await;
    insert_verify_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "status": "error" }),
    )
    .await;
    insert_verify_cache(
        &state,
        "ws-b/container/claude-crp",
        serde_json::json!({ "status": "ok" }),
    )
    .await;

    let Json(config) = select_provider_harness_source(
        State(ctx_daemon::daemon::DaemonHandle::new(Arc::clone(&state)).providers()),
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

    let (codex_options_cached, claude_options_cached) = state
        .test_with_provider_options_cache(|cache| {
            (
                cache.contains_key("ws-a/host/codex"),
                cache.contains_key("ws-b/container/claude-crp"),
            )
        })
        .await;
    assert!(!codex_options_cached);
    assert!(claude_options_cached);

    let (codex_verify_cached, claude_verify_cached) = state
        .test_with_provider_verify_cache(|cache| {
            (
                cache.contains_key("ws-a/host/codex"),
                cache.contains_key("ws-b/container/claude-crp"),
            )
        })
        .await;
    assert!(!codex_verify_cached);
    assert!(claude_verify_cached);
}
