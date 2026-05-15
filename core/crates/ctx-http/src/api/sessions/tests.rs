use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use ctx_daemon::test_support::TestDaemon;
use ctx_providers::fake::FakeProviderAdapter;

#[path = "tests/title_generation.rs"]
mod title_generation_tests;

async fn setup_state() -> (tempfile::TempDir, TestDaemon, Session) {
    let data_dir = tempfile::tempdir().unwrap();
    let providers = HashMap::from([(
        "fake".to_string(),
        Arc::new(FakeProviderAdapter::new()) as Arc<dyn ctx_providers::adapters::ProviderAdapter>,
    )]);
    let daemon = TestDaemon::new_with_providers_for_test(
        data_dir.path().to_path_buf(),
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    )
    .await
    .unwrap();
    let session = daemon
        .seed_title_generation_session_for_test(data_dir.path())
        .await
        .unwrap();

    (data_dir, daemon, session)
}
