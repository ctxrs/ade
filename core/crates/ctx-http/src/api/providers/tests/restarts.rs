use super::*;

#[derive(Default)]
struct RestartTrackingAdapter {
    restart_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderAdapter for RestartTrackingAdapter {
    async fn inspect(&self) -> anyhow::Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("test".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
        _hooks: ctx_providers::adapters::ProviderRunHooks,
    ) -> anyhow::Result<RunHandle> {
        anyhow::bail!("run not used in this test")
    }

    async fn cancel(&self, _handle: &mut RunHandle) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        Vec::new()
    }

    async fn restart(&self, _reason: &str, _mode: ProviderRestartMode) -> anyhow::Result<()> {
        self.restart_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn supports_restart_mode(&self, _mode: ProviderRestartMode) -> bool {
        true
    }
}

#[derive(Default)]
struct RestartFailingAdapter {
    restart_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderAdapter for RestartFailingAdapter {
    async fn inspect(&self) -> anyhow::Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("test".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
        _hooks: ctx_providers::adapters::ProviderRunHooks,
    ) -> anyhow::Result<RunHandle> {
        anyhow::bail!("run not used in this test")
    }

    async fn cancel(&self, _handle: &mut RunHandle) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        Vec::new()
    }

    async fn restart(&self, _reason: &str, _mode: ProviderRestartMode) -> anyhow::Result<()> {
        self.restart_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("restart failed")
    }

    fn supports_restart_mode(&self, _mode: ProviderRestartMode) -> bool {
        true
    }
}

struct UnsupportedRestartAdapter;

#[async_trait::async_trait]
impl ProviderAdapter for UnsupportedRestartAdapter {
    async fn inspect(&self) -> anyhow::Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("test".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ctx_providers::adapters::ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
        _hooks: ctx_providers::adapters::ProviderRunHooks,
    ) -> anyhow::Result<RunHandle> {
        anyhow::bail!("run not used in this test")
    }

    async fn cancel(&self, _handle: &mut RunHandle) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        Vec::new()
    }
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
