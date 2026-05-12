use super::support::{RestartFailingAdapter, RestartTrackingAdapter, UnsupportedRestartAdapter};
use super::*;

async fn insert_options_cache(state: &Arc<AppState>, key: &str, value: serde_json::Value) {
    state
        .providers
        .with_provider_options_cache(|cache| {
            cache.insert(
                key.to_string(),
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value,
                },
            );
        })
        .await;
}

async fn insert_verify_cache(state: &Arc<AppState>, key: &str, value: serde_json::Value) {
    state
        .providers
        .with_provider_verify_cache(|cache| {
            cache.insert(
                key.to_string(),
                crate::daemon::CachedProviderVerify {
                    cached_at: std::time::Instant::now(),
                    value,
                },
            );
        })
        .await;
}

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

    restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect("restart should succeed");

    let (codex_options_cached, claude_options_cached) = state
        .providers
        .with_provider_options_cache(|cache| {
            (
                cache.contains_key("ws-a/host/codex"),
                cache.contains_key("ws-b/container/claude-crp"),
            )
        })
        .await;
    assert!(!codex_options_cached);
    assert!(claude_options_cached);

    let (codex_verify_cached, claude_verify_cached) = state
        .providers
        .with_provider_verify_cache(|cache| {
            (
                cache.contains_key("ws-a/host/codex"),
                cache.contains_key("ws-b/container/claude-crp"),
            )
        })
        .await;
    assert!(!codex_verify_cached);
    assert!(claude_verify_cached);

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

    insert_options_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
    )
    .await;

    let err = restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect_err("restart failure should bubble up");
    assert!(err
        .to_string()
        .contains("provider auth updated but drain-restart failed for codex"));

    let options_cached = state
        .providers
        .with_provider_options_cache(|cache| cache.contains_key("ws-a/host/codex"))
        .await;
    assert!(!options_cached);

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

    insert_options_cache(
        &state,
        "ws-a/host/codex",
        serde_json::json!({ "provider_id": "codex", "probe_ok": false }),
    )
    .await;

    restart_provider_for_auth_change(&state, "codex", "test auth updated")
        .await
        .expect("unsupported restart should be skipped");

    let options_cached = state
        .providers
        .with_provider_options_cache(|cache| cache.contains_key("ws-a/host/codex"))
        .await;
    assert!(!options_cached);
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
