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

    let workspace = stores
        .global()
        .create_workspace(
            "ws".to_string(),
            data_dir.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = stores.workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            data_dir.path().to_string_lossy().to_string(),
            "base".to_string(),
            None,
        )
        .await
        .unwrap();
    stores
        .global()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .unwrap();
    let task = store
        .create_task(
            workspace.id,
            title_generation::DEFAULT_SESSION_TITLE.to_string(),
            None,
        )
        .await
        .unwrap();
    stores
        .global()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "fake-model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    stores
        .global()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();

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

    (data_dir, daemon, session)
}
