use super::*;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderProcessInfo, ProviderRestartMode, ProviderStatus,
    RunHandle, TurnInput,
};
use ctx_store::StoreManager;
use std::collections::HashMap;
use std::path::PathBuf;

#[tokio::test]
async fn codex_login_persistence_requires_auth_file() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let account_id = "acct-missing-auth";
    provider_accounts::ensure_codex_account_dir(&state.core.data_root, account_id)
        .await
        .unwrap();

    let err =
        persist_successful_codex_login(&state, account_id, "Missing Auth".to_string(), None, None)
            .await
            .unwrap_err();

    assert!(err
        .to_string()
        .contains("missing persisted codex auth file"));
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    assert!(registry.accounts.is_empty());
    assert!(registry.active_account_id.is_none());
}

struct RestartFailingAdapter;

#[async_trait::async_trait]
impl ProviderAdapter for RestartFailingAdapter {
    async fn inspect(&self) -> anyhow::Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "codex-crp".to_string(),
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
        anyhow::bail!("restart failed")
    }
}

#[tokio::test]
async fn codex_login_persistence_rolls_back_when_restart_fails() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::from([(
            "codex-crp".to_string(),
            Arc::new(RestartFailingAdapter) as Arc<dyn ProviderAdapter>,
        )]),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let account_id = "acct-restart-fails";
    let account_dir =
        provider_accounts::ensure_codex_account_dir(&state.core.data_root, account_id)
            .await
            .unwrap();
    tokio::fs::write(
        account_dir.join("auth.json"),
        "{\"access_token\":\"token\"}",
    )
    .await
    .unwrap();

    let err = persist_successful_codex_login(
        &state,
        account_id,
        "Restart Fails".to_string(),
        Some("restart@example.com".to_string()),
        None,
    )
    .await
    .expect_err("restart failure should bubble up");
    assert!(!err.to_string().is_empty());

    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    assert!(registry.accounts.is_empty());
    assert!(registry.active_account_id.is_none());
}
