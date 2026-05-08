use super::support::{RestartFailingAdapter, RestartTrackingAdapter, UnsupportedRestartAdapter};
use super::*;

#[tokio::test]
async fn restart_provider_for_auth_change_invalidates_only_matching_provider_probe_caches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let adapter = Arc::new(RestartTrackingAdapter::default());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::from([(
            "codex".to_string(),
            adapter.clone() as Arc<dyn ProviderAdapter>,
        )]),
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

    restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect("restart should succeed");

    let options_cache = state.providers.options_cache.lock().await;
    assert!(!options_cache.contains_key("ws-a/host/codex"));
    assert!(options_cache.contains_key("ws-b/container/claude-crp"));
    drop(options_cache);

    let verify_cache = state.providers.verify_cache.lock().await;
    assert!(!verify_cache.contains_key("ws-a/host/codex"));
    assert!(verify_cache.contains_key("ws-b/container/claude-crp"));
    drop(verify_cache);

    assert_eq!(adapter.restart_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn restart_provider_for_auth_change_returns_error_when_adapter_restart_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let adapter = Arc::new(RestartFailingAdapter::default());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::from([(
            "codex".to_string(),
            adapter.clone() as Arc<dyn ProviderAdapter>,
        )]),
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

    let err = restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect_err("restart failure should bubble up");
    assert!(err
        .to_string()
        .contains("provider auth updated but drain-restart failed for codex"));

    let options_cache = state.providers.options_cache.lock().await;
    assert!(!options_cache.contains_key("ws-a/host/codex"));
    drop(options_cache);

    assert_eq!(adapter.restart_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn restart_provider_for_auth_change_skips_adapters_without_drain_restart() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::from([(
            "codex".to_string(),
            Arc::new(UnsupportedRestartAdapter) as Arc<dyn ProviderAdapter>,
        )]),
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

    restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect("unsupported restart should be skipped");

    let options_cache = state.providers.options_cache.lock().await;
    assert!(!options_cache.contains_key("ws-a/host/codex"));
}

#[tokio::test]
async fn set_codex_active_account_returns_error_when_restart_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::from([(
            "codex".to_string(),
            Arc::new(RestartFailingAdapter::default()) as Arc<dyn ProviderAdapter>,
        )]),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));
    provider_accounts::upsert_codex_account(
        &state.core.data_root,
        provider_accounts::CodexAccountEntry {
            id: "acct".to_string(),
            label: "Account".to_string(),
            kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
            email: Some("acct@example.com".to_string()),
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
        },
    )
    .await
    .expect("seed codex account");

    let err = set_codex_active_account(
        State(Arc::clone(&state)),
        Json(CodexActiveAccountReq {
            account_id: Some("acct".to_string()),
        }),
    )
    .await
    .expect_err("restart failure should surface");
    assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err
        .1
         .0
        .error
        .contains("provider auth updated but drain-restart failed"));
}
