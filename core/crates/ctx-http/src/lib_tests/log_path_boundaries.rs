use super::*;

use chrono::Utc;
use ctx_core::ids::{MergeQueueEntryId, MergeQueueRunId};
use ctx_core::models::{
    MergeQueueEntry, MergeQueueEntryStatus, MergeQueuePatchSource, MergeQueueRun,
    MergeQueueRunStatus, WorktreeBootstrapStatus,
};
use ctx_store::WorktreeBootstrapResultUpdate;

mod merge_queue;
mod worktree_bootstrap;

struct LogPathFixture {
    _home_lock: tokio::sync::MutexGuard<'static, ()>,
    _home: EnvVarGuard,
    _home_dir: tempfile::TempDir,
    data_dir: tempfile::TempDir,
    git_repo: tempfile::TempDir,
    app: axum::Router,
    state: Arc<AppState>,
    workspace: ctx_core::models::Workspace,
}

async fn build_log_path_fixture() -> LogPathFixture {
    let home_lock = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home_dir = tempfile::tempdir().unwrap();
    let home = EnvVarGuard::set("HOME", &home_dir.path().to_string_lossy());

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

    LogPathFixture {
        _home_lock: home_lock,
        _home: home,
        _home_dir: home_dir,
        data_dir,
        git_repo,
        app,
        state,
        workspace,
    }
}

async fn create_task_with_primary_session(
    fixture: &LogPathFixture,
) -> (ctx_core::models::Task, ctx_core::models::Session) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", fixture.workspace.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = fixture.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();
    let session = load_primary_session_via_api(&fixture.app, &task).await;

    (task, session)
}

async fn bootstrap_logs_response(
    app: &axum::Router,
    worktree_id: ctx_core::ids::WorktreeId,
) -> axum::response::Response {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/worktrees/{}/bootstrap/logs", worktree_id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    res
}

fn merge_queue_entry(workspace_id: ctx_core::ids::WorkspaceId, message: &str) -> MergeQueueEntry {
    let now = Utc::now();
    MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some(message.to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some("base".to_string()),
        head_commit_sha: Some("head".to_string()),
        patch_path: "/tmp/log-path-boundary.patch".to_string(),
        patch_size: 1,
        status: MergeQueueEntryStatus::Failed,
        result_commit_sha: None,
        error_message: Some("failed".to_string()),
        created_at: now,
        updated_at: now,
    }
}

fn merge_queue_run(
    entry_id: MergeQueueEntryId,
    log_path: String,
    error_message: &str,
) -> MergeQueueRun {
    let now = Utc::now();
    MergeQueueRun {
        id: MergeQueueRunId::new(),
        entry_id,
        status: MergeQueueRunStatus::Failed,
        started_at: now,
        finished_at: Some(now),
        exit_code: Some(1),
        log_path: Some(log_path),
        error_message: Some(error_message.to_string()),
        result_commit_sha: None,
    }
}

async fn merge_queue_logs_response(
    app: &axum::Router,
    workspace_id: ctx_core::ids::WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> axum::response::Response {
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/merge_queue/entries/{}/logs",
            workspace_id.0, entry_id.0
        ))
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}
