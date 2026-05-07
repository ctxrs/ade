use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use axum::http::{Method, Request};
use ctx_core::ids::TaskId;
use ctx_core::models::{Session, Task, VcsKind};
use ctx_http::daemon::AppState;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderRecommendedAction, ProviderUsability, ProviderUsabilityStatus,
};
use ctx_providers::fake::FakeProviderAdapter;
use serde_json::{json, Value};
use tokio::time::{timeout, Duration};

mod common;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn lock_test() -> tokio::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().await
}

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

async fn setup_state(data_root: &std::path::Path, prewarm_statuses: bool) -> Arc<AppState> {
    let stores = common::setup_store(data_root).await;
    let state = common::build_state(
        data_root.to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    if !prewarm_statuses {
        return state;
    }
    let mut status = FakeProviderAdapter::new().inspect().await.unwrap();
    status.usability = ProviderUsability {
        usable: true,
        status: ProviderUsabilityStatus::Ready,
        reason_code: None,
        reason: None,
        blocking_provider_ids: Vec::new(),
        recommended_action: ProviderRecommendedAction::None,
    };
    state
        .providers
        .statuses
        .lock()
        .await
        .insert("fake".into(), status);
    state
}

#[tokio::test]
async fn create_task_creates_default_session_when_requested() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let (status, task): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(json!({ "title": "task" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let session_id = task
        .primary_session_id
        .expect("default session should be created");
    let worktree_id = task
        .primary_worktree_id
        .expect("default worktree should be created");

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert_eq!(sessions.len(), 1, "expected exactly one default session");
    assert_eq!(sessions[0].id, session_id);
    assert_eq!(sessions[0].worktree_id, worktree_id);
    assert_eq!(sessions[0].provider_id, "fake");
    assert_eq!(sessions[0].model_id, "fake-model");

    let persisted_task = store.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(persisted_task.primary_session_id, Some(session_id));
    assert_eq!(persisted_task.primary_worktree_id, Some(worktree_id));
}

#[tokio::test]
async fn create_task_creates_default_session_without_prewarmed_provider_statuses() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), false).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let (status, task): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(json!({ "title": "cold-start task" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(task.primary_session_id.is_some());
    assert!(task.primary_worktree_id.is_some());

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert_eq!(sessions.len(), 1, "expected exactly one default session");
    assert_eq!(sessions[0].provider_id, "fake");
    assert_eq!(sessions[0].model_id, "fake-model");
}

#[tokio::test]
async fn create_task_rejects_legacy_create_default_session_flag() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/workspaces/{}/tasks", workspace.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "legacy bare task",
                "create_default_session": false,
            })
            .to_string(),
        ))
        .unwrap();
    let (status, _body) = common::oneshot_bytes(&app, req).await;

    assert!(
        status.is_client_error(),
        "legacy task-create field must be rejected, got {status}"
    );
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let tasks = store.list_tasks(workspace.id).await.unwrap();
    assert!(
        tasks.is_empty(),
        "rejected legacy task-create field must not persist a task"
    );
}

#[tokio::test]
async fn create_session_rejects_second_top_level_session_for_task() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let (task_status, task): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(json!({ "title": "single primary task" })),
    )
    .await;
    assert_eq!(task_status, StatusCode::OK);
    assert!(
        task.primary_session_id.is_some(),
        "task creation must create the primary session"
    );

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "provider_id": "fake",
                "model_id": "fake-model",
            })
            .to_string(),
        ))
        .unwrap();
    let (session_status, _body) = common::oneshot_bytes(&app, req).await;
    assert_eq!(session_status, StatusCode::CONFLICT);

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert_eq!(
        sessions.len(),
        1,
        "failed second top-level session create must not create another session"
    );
    assert_eq!(Some(sessions[0].id), task.primary_session_id);
}

#[tokio::test]
async fn create_task_replay_with_same_id_does_not_create_extra_default_sessions() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let task_id = uuid::Uuid::new_v4().to_string();
    let uri = format!("/api/workspaces/{}/tasks", workspace.id.0);
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let request_body = json!({
        "id": task_id,
        "title": "replayed task",
    });
    let app_a = app.clone();
    let app_b = app.clone();
    let ((status_a, task_a), (status_b, task_b)): ((StatusCode, Task), (StatusCode, Task)) = tokio::join!(
        common::json_request(
            &app_a,
            axum::http::Method::POST,
            uri.clone(),
            Some(request_body.clone()),
        ),
        common::json_request(&app_b, axum::http::Method::POST, uri, Some(request_body)),
    );

    assert_eq!(status_a, StatusCode::OK);
    assert_eq!(status_b, StatusCode::OK);
    assert_eq!(task_a.id, task_b.id);
    assert_eq!(task_a.primary_session_id, task_b.primary_session_id);
    assert_eq!(task_a.primary_worktree_id, task_b.primary_worktree_id);
    assert!(task_a.primary_session_id.is_some());
    assert!(task_a.primary_worktree_id.is_some());

    let sessions = store.list_sessions_for_task(task_a.id).await.unwrap();
    assert_eq!(sessions.len(), 1, "expected exactly one default session");
    let worktrees = store.list_worktrees(workspace.id).await.unwrap();
    assert_eq!(
        worktrees.len(),
        1,
        "expected exactly one provisioned worktree"
    );
}

#[tokio::test]
async fn create_task_replay_validates_requested_default_session() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let task_id = uuid::Uuid::new_v4().to_string();
    let session_id = uuid::Uuid::new_v4().to_string();
    let uri = format!("/api/workspaces/{}/tasks", workspace.id.0);
    let request_body = json!({
        "id": task_id,
        "title": "replayed explicit default",
        "default_session": {
            "id": session_id,
            "provider_id": "fake",
            "model_id": "fake-model",
        }
    });

    let (status_a, task_a): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        uri.clone(),
        Some(request_body.clone()),
    )
    .await;
    assert_eq!(status_a, StatusCode::OK);
    assert_eq!(
        task_a.primary_session_id.map(|id| id.0.to_string()),
        Some(session_id.clone())
    );

    let (status_b, task_b): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        uri.clone(),
        Some(request_body),
    )
    .await;
    assert_eq!(status_b, StatusCode::OK);
    assert_eq!(task_b.id, task_a.id);
    assert_eq!(task_b.primary_session_id, task_a.primary_session_id);

    let (conflict_status, _body): (StatusCode, Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        uri,
        Some(json!({
            "id": task_id,
            "title": "replayed explicit default",
            "default_session": {
                "id": uuid::Uuid::new_v4().to_string(),
                "provider_id": "fake",
                "model_id": "fake-model",
            }
        })),
    )
    .await;
    assert_eq!(conflict_status, StatusCode::CONFLICT);

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task_a.id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, task_a.primary_session_id.unwrap());
}

#[tokio::test]
async fn create_task_replay_allows_server_generated_default_session_id() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let task_id = uuid::Uuid::new_v4().to_string();
    let uri = format!("/api/workspaces/{}/tasks", workspace.id.0);
    let request_body = json!({
        "id": task_id,
        "title": "replayed implicit default id",
        "default_session": {
            "provider_id": "fake",
            "model_id": "fake-model",
        }
    });

    let (status_a, task_a): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        uri.clone(),
        Some(request_body.clone()),
    )
    .await;
    assert_eq!(status_a, StatusCode::OK);
    assert!(task_a.primary_session_id.is_some());

    let (status_b, task_b): (StatusCode, Task) =
        common::json_request(&app, axum::http::Method::POST, uri, Some(request_body)).await;
    assert_eq!(status_b, StatusCode::OK);
    assert_eq!(task_b.id, task_a.id);
    assert_eq!(task_b.primary_session_id, task_a.primary_session_id);
    assert_eq!(task_b.primary_worktree_id, task_a.primary_worktree_id);

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let sessions = store.list_sessions_for_task(task_a.id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(Some(sessions[0].id), task_a.primary_session_id);
}

#[tokio::test]
async fn create_task_in_non_repo_workspace_returns_bad_request() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let workspace_root = tempfile::tempdir().unwrap();
    std::fs::write(workspace_root.path().join("README.md"), "hello\n").unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(json!({ "title": "bad repo task" })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| !error.is_empty()));

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let tasks = store.list_tasks(workspace.id).await.unwrap();
    assert!(
        tasks.is_empty(),
        "failed default-session preflight must not persist a task"
    );
}

#[tokio::test]
async fn create_task_rolls_back_if_default_session_preflight_fails_after_task_persist() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();

    let task_uuid = common::fixed_uuid(0xfeed);
    let task_id = TaskId(task_uuid);
    let creation_lock = state.sessions.task_session_creation_lock(task_id).await;
    let creation_guard = creation_lock.lock_owned().await;

    let request_app = app.clone();
    let request_uri = format!("/api/workspaces/{}/tasks", workspace.id.0);
    let request = tokio::spawn(async move {
        common::json_request::<Value>(
            &request_app,
            axum::http::Method::POST,
            request_uri,
            Some(json!({
                "id": task_uuid.to_string(),
                "title": "task that loses repo state",
            })),
        )
        .await
    });
    timeout(Duration::from_secs(5), async {
        loop {
            if store.get_task(task_id).await.unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("request should persist the task before waiting on the session lock");

    state
        .global_store()
        .delete_workspace_task_index(task_id)
        .await
        .unwrap();
    std::fs::remove_dir_all(repo.path().join(".git")).unwrap();
    drop(creation_guard);

    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| !error.is_empty()));

    let tasks = store.list_tasks(workspace.id).await.unwrap();
    assert!(
        tasks.is_empty(),
        "task should be rolled back if the second-phase default-session preflight fails"
    );
    let worktrees = store.list_worktrees(workspace.id).await.unwrap();
    assert!(
        worktrees.is_empty(),
        "second-phase preflight failure must not leave behind worktrees"
    );
}

#[tokio::test]
async fn create_session_waits_for_task_session_creation_lock() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let task = store
        .create_task(workspace.id, "locked session".to_string(), None)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();

    let creation_lock = state.sessions.task_session_creation_lock(task.id).await;
    let creation_guard = creation_lock.lock_owned().await;

    let request_app = app.clone();
    let request = tokio::spawn(async move {
        common::json_request::<Session>(
            &request_app,
            axum::http::Method::POST,
            format!("/api/tasks/{}/sessions", task.id.0),
            Some(json!({ "provider_id": "fake", "model_id": "fake-model" })),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert!(
        sessions.is_empty(),
        "session creation should wait behind the per-task session creation lock"
    );

    drop(creation_guard);

    let (status, session) = request.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session.task_id, task.id);

    let sessions = store.list_sessions_for_task(task.id).await.unwrap();
    assert_eq!(
        sessions.len(),
        1,
        "session should be created once the lock is released"
    );
}

#[tokio::test]
async fn concurrent_replayed_create_task_failures_return_validation_error_not_not_found() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();

    let task_uuid = common::fixed_uuid(0xbeef);
    let task_id = TaskId(task_uuid);
    let creation_lock = state.sessions.task_session_creation_lock(task_id).await;
    let creation_guard = creation_lock.lock_owned().await;

    let uri = format!("/api/workspaces/{}/tasks", workspace.id.0);
    let body = json!({
        "id": task_uuid.to_string(),
        "title": "replayed failure task",
    });
    let uri_a = uri.clone();
    let body_a = body.clone();
    let app_a = app.clone();
    let app_b = app.clone();
    let request_a = tokio::spawn(async move {
        common::json_request::<Value>(&app_a, axum::http::Method::POST, uri_a, Some(body_a)).await
    });
    timeout(Duration::from_secs(5), async {
        loop {
            if store.get_task(task_id).await.unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first replay request should persist the shared task before waiting on the lock");

    let request_b = tokio::spawn(async move {
        common::json_request::<Value>(&app_b, axum::http::Method::POST, uri, Some(body)).await
    });

    std::fs::remove_dir_all(repo.path().join(".git")).unwrap();
    drop(creation_guard);

    let (result_a, result_b) = tokio::join!(request_a, request_b);
    let (status_a, body_a) = result_a.unwrap();
    let (status_b, body_b) = result_b.unwrap();

    assert_eq!(status_a, StatusCode::BAD_REQUEST);
    assert_eq!(status_b, StatusCode::BAD_REQUEST);
    for body in [&body_a, &body_b] {
        assert!(body
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|error| !error.is_empty()));
        assert_ne!(
            body.get("error").and_then(Value::as_str),
            Some("task not found"),
            "replayed failure should report the validation failure, not a synthetic 404"
        );
    }

    let tasks = store.list_tasks(workspace.id).await.unwrap();
    assert!(
        tasks.is_empty(),
        "both failed replay requests should leave no persisted task behind"
    );
    let worktrees = store.list_worktrees(workspace.id).await.unwrap();
    assert!(
        worktrees.is_empty(),
        "failed replay requests must not leak worktrees"
    );
}

#[tokio::test]
async fn concurrent_replayed_create_task_with_different_payload_conflicts() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();

    let task_uuid = common::fixed_uuid(0xc0de);
    let task_id = TaskId(task_uuid);
    let creation_lock = state.sessions.task_session_creation_lock(task_id).await;
    let creation_guard = creation_lock.lock_owned().await;

    let uri = format!("/api/workspaces/{}/tasks", workspace.id.0);
    let app_a = app.clone();
    let request_a = tokio::spawn(async move {
        common::json_request::<Value>(
            &app_a,
            axum::http::Method::POST,
            uri.clone(),
            Some(json!({
                "id": task_uuid.to_string(),
                "title": "canonical replay task",
            })),
        )
        .await
    });
    timeout(Duration::from_secs(5), async {
        loop {
            if store.get_task(task_id).await.unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first replay request should persist the shared task before waiting on the lock");

    let app_b = app.clone();
    let request_b = tokio::spawn(async move {
        common::json_request::<Value>(
            &app_b,
            axum::http::Method::POST,
            format!("/api/workspaces/{}/tasks", workspace.id.0),
            Some(json!({
                "id": task_uuid.to_string(),
                "title": "different replay title",
            })),
        )
        .await
    });

    std::fs::remove_dir_all(repo.path().join(".git")).unwrap();
    drop(creation_guard);

    let (result_a, result_b) = tokio::join!(request_a, request_b);
    let (status_a, body_a) = result_a.unwrap();
    let (status_b, body_b) = result_b.unwrap();

    assert_eq!(status_a, StatusCode::BAD_REQUEST);
    assert_eq!(status_b, StatusCode::CONFLICT);
    assert!(body_a
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| !error.is_empty()));
    assert_eq!(
        body_b.get("error").and_then(Value::as_str),
        Some("task id already exists")
    );

    let tasks = store.list_tasks(workspace.id).await.unwrap();
    assert!(
        tasks.is_empty(),
        "conflicting replay should not leave behind a task after the canonical request fails"
    );
    let worktrees = store.list_worktrees(workspace.id).await.unwrap();
    assert!(
        worktrees.is_empty(),
        "conflicting replay must not create worktrees"
    );
}

#[tokio::test]
async fn conflicting_session_id_does_not_leak_new_worktree() {
    let _test_lock = lock_test().await;
    let _show_fake = EnvVarGuard::set("CTX_SHOW_FAKE_PROVIDER", "1");
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = setup_state(data_dir.path(), true).await;
    let app = common::router(Arc::clone(&state));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let (existing_status, existing_task): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(json!({ "title": "existing session owner" })),
    )
    .await;
    assert_eq!(existing_status, StatusCode::OK);
    let existing_session_id = existing_task
        .primary_session_id
        .expect("existing task should have a default session");

    let (target_status, target_task): (StatusCode, Task) = common::json_request(
        &app,
        axum::http::Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(json!({ "title": "target task" })),
    )
    .await;
    assert_eq!(target_status, StatusCode::OK);
    let target_primary_session_id = target_task
        .primary_session_id
        .expect("target task should have a default session");
    let target_primary_worktree_id = target_task
        .primary_worktree_id
        .expect("target task should have a default worktree");

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree_count_before = store.list_worktrees(workspace.id).await.unwrap().len();

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/tasks/{}/sessions", target_task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "id": existing_session_id.0.to_string(),
                "provider_id": "fake",
                "model_id": "fake-model",
                "parent_session_id": target_primary_session_id.0.to_string(),
                "relationship": "sub_agent",
            })
            .to_string(),
        ))
        .unwrap();
    let (status, body) = common::oneshot_bytes(&app, req).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body.is_empty(),
        "session conflict route should not return a JSON body"
    );

    let sessions = store.list_sessions_for_task(target_task.id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, target_primary_session_id);
    let refreshed_target = store.get_task(target_task.id).await.unwrap().unwrap();
    assert_eq!(
        refreshed_target.primary_worktree_id,
        Some(target_primary_worktree_id),
        "conflicting explicit child session id must not change the target task primary worktree"
    );
    let worktree_count_after = store.list_worktrees(workspace.id).await.unwrap().len();
    assert_eq!(
        worktree_count_after, worktree_count_before,
        "conflicting explicit session id must not leak a newly provisioned worktree"
    );
}
