use super::*;

use chrono::Utc;
use ctx_core::ids::{MergeQueueEntryId, MergeQueueRunId};
use ctx_core::models::{
    MergeQueueEntry, MergeQueueEntryStatus, MergeQueuePatchSource, MergeQueueRun,
    MergeQueueRunStatus, WorktreeBootstrapStatus,
};
use ctx_store::WorktreeBootstrapResultUpdate;

#[tokio::test]
async fn worktree_bootstrap_logs_return_in_root_log_file() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", workspace.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let session = load_primary_session_via_api(&app, &task).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("workspace worktree");

    let log_dir = crate::logs::logs_dir(data_dir.path()).join("worktree-bootstrap");
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("bootstrap.log");
    std::fs::write(&log_path, b"inside bootstrap log\n").unwrap();

    store
        .update_worktree_bootstrap_result(WorktreeBootstrapResultUpdate {
            worktree_id: worktree.id,
            status: WorktreeBootstrapStatus::Success,
            started_at: Utc::now(),
            finished_at: Utc::now(),
            exit_code: Some(0),
            timeout_sec: Some(60),
            error: None,
            log_path: Some(log_path.to_string_lossy().to_string()),
            log_truncated: Some(false),
            command: Some("true".to_string()),
            script_path: None,
        })
        .await
        .unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/worktrees/{}/bootstrap/logs", worktree.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"inside bootstrap log\n");
}

#[tokio::test]
async fn worktree_bootstrap_logs_fail_closed_for_legacy_outside_paths() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", workspace.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let session = load_primary_session_via_api(&app, &task).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("workspace worktree");

    let outside_dir = tempfile::tempdir().unwrap();
    let outside_path = outside_dir.path().join("bootstrap.log");
    std::fs::write(&outside_path, b"outside bootstrap log\n").unwrap();

    store
        .update_worktree_bootstrap_result(WorktreeBootstrapResultUpdate {
            worktree_id: worktree.id,
            status: WorktreeBootstrapStatus::Failed,
            started_at: Utc::now(),
            finished_at: Utc::now(),
            exit_code: Some(1),
            timeout_sec: Some(60),
            error: Some("legacy outside path".to_string()),
            log_path: Some(outside_path.to_string_lossy().to_string()),
            log_truncated: Some(false),
            command: Some("false".to_string()),
            script_path: None,
        })
        .await
        .unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/worktrees/{}/bootstrap/logs", worktree.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn merge_queue_entry_logs_return_in_root_log_file() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();

    let now = Utc::now();
    let entry = MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id: workspace.id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some("inside log".to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some("base".to_string()),
        head_commit_sha: Some("head".to_string()),
        patch_path: "/tmp/inside.patch".to_string(),
        patch_size: 1,
        status: MergeQueueEntryStatus::Failed,
        result_commit_sha: None,
        error_message: Some("failed".to_string()),
        created_at: now,
        updated_at: now,
    };
    store.create_merge_queue_entry(&entry).await.unwrap();

    let log_dir = git_repo
        .path()
        .join(".ctx")
        .join("merge-queue")
        .join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("merge-queue.log");
    std::fs::write(&log_path, b"inside merge queue log\n").unwrap();

    let run = MergeQueueRun {
        id: MergeQueueRunId::new(),
        entry_id: entry.id,
        status: MergeQueueRunStatus::Failed,
        started_at: now,
        finished_at: Some(now),
        exit_code: Some(1),
        log_path: Some(log_path.to_string_lossy().to_string()),
        error_message: Some("inside path".to_string()),
        result_commit_sha: None,
    };
    store.create_merge_queue_run(&run).await.unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/merge_queue/entries/{}/logs",
            workspace.id.0, entry.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"inside merge queue log\n");
}

#[tokio::test]
async fn merge_queue_entry_logs_fail_closed_for_legacy_outside_paths() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();

    let now = Utc::now();
    let entry = MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id: workspace.id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some("legacy outside log".to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some("base".to_string()),
        head_commit_sha: Some("head".to_string()),
        patch_path: "/tmp/legacy-outside.patch".to_string(),
        patch_size: 1,
        status: MergeQueueEntryStatus::Failed,
        result_commit_sha: None,
        error_message: Some("failed".to_string()),
        created_at: now,
        updated_at: now,
    };
    store.create_merge_queue_entry(&entry).await.unwrap();

    let outside_dir = tempfile::tempdir().unwrap();
    let outside_path = outside_dir.path().join("merge-queue.log");
    std::fs::write(&outside_path, b"outside merge queue log\n").unwrap();

    let run = MergeQueueRun {
        id: MergeQueueRunId::new(),
        entry_id: entry.id,
        status: MergeQueueRunStatus::Failed,
        started_at: now,
        finished_at: Some(now),
        exit_code: Some(1),
        log_path: Some(outside_path.to_string_lossy().to_string()),
        error_message: Some("legacy outside path".to_string()),
        result_commit_sha: None,
    };
    store.create_merge_queue_run(&run).await.unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/merge_queue/entries/{}/logs",
            workspace.id.0, entry.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
