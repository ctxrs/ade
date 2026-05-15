use super::*;
use std::collections::HashMap;

use ctx_daemon::test_support::TestDaemon;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

#[path = "tests/title_generation.rs"]
mod title_generation_tests;

async fn setup_state() -> (tempfile::TempDir, TestDaemon, Session) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let daemon = TestDaemon::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    );
    let session = daemon
        .seed_title_generation_session_for_test(data_dir.path())
        .await
        .unwrap();

    (data_dir, daemon, session)
}
